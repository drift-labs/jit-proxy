import { JitProxyClient, PriceType } from './jitProxyClient';
import { PublicKey } from '@solana/web3.js';
import {
	AuctionSubscriber,
	BN, BulkAccountLoader,
	DriftClient,
	getUserStatsAccountPublicKey,
	hasAuctionPrice,
	isVariant,
	Order,
	UserAccount, UserMap, UserStatsMap,
} from '@drift-labs/sdk';

export type UserFilter = (
	userAccount: UserAccount,
	userKey: string,
	order: Order
) => boolean;

export type JitParams = {
	bid: BN;
	ask: BN;
	minPosition: BN;
	maxPosition;
	priceType: PriceType;
	subAccountId?: number;
};

export class Jitter {
	auctionSubscriber: AuctionSubscriber;
	driftClient: DriftClient;
	jitProxyClient: JitProxyClient;
	userStatsMap: UserStatsMap;

	perpParams = new Map<number, JitParams>();
	spotParams = new Map<number, JitParams>();

	onGoingAuctions = new Map<string, Promise<void>>();

	userFilter: UserFilter;

	constructor({
		auctionSubscriber,
		jitProxyClient,
		driftClient,
		userStatsMap,
	}: {
		driftClient: DriftClient;
		auctionSubscriber: AuctionSubscriber;
		jitProxyClient: JitProxyClient;
		userStatsMap?: UserStatsMap;
	}) {
		this.auctionSubscriber = auctionSubscriber;
		this.driftClient = driftClient;
		this.jitProxyClient = jitProxyClient;
		this.userStatsMap = userStatsMap || new UserStatsMap(this.driftClient, {
			type: 'polling',
			accountLoader: new BulkAccountLoader(this.driftClient.connection, 'confirmed', 0),
		});
	}

	async subscribe(): Promise<void> {
		await this.driftClient.subscribe();
		await this.userStatsMap.subscribe();

		await this.auctionSubscriber.subscribe();
		this.auctionSubscriber.eventEmitter.on(
			'onAccountUpdate',
			async (taker, takerKey, slot) => {
				const takerKeyString = takerKey.toBase58();

				const takerStatsKey = getUserStatsAccountPublicKey(
					this.driftClient.program.programId,
					taker.authority
				);
				for (const order of taker.orders) {
					if (!isVariant(order.status, 'open')) {
						continue;
					}

					if (!hasAuctionPrice(order, slot)) {
						continue;
					}

					if (this.userFilter) {
						if (this.userFilter(taker, takerKeyString, order)) {
							return;
						}
					}

					const orderSignature = this.getOrderSignatures(
						takerKeyString,
						order.orderId
					);
					if (this.onGoingAuctions.has(orderSignature)) {
						continue;
					}

					if (isVariant(order.marketType, 'perp')) {
						if (!this.perpParams.has(order.marketIndex)) {
							return;
						}

						const promise = this.createTryFill(
							taker,
							takerKey,
							takerStatsKey,
							order,
							orderSignature
						).bind(this)();
						this.onGoingAuctions.set(orderSignature, promise);
					} else {
						if (!this.spotParams.has(order.marketIndex)) {
							return;
						}

						const promise = this.createTryFill(
							taker,
							takerKey,
							takerStatsKey,
							order,
							orderSignature
						).bind(this)();
						this.onGoingAuctions.set(orderSignature, promise);
					}
				}
			}
		);
	}

	createTryFill(
		taker: UserAccount,
		takerKey: PublicKey,
		takerStatsKey: PublicKey,
		order: Order,
		orderSignature: string
	): () => Promise<void> {
		return async () => {
			let i = 0;
			while (i < 10) {
				const params = this.perpParams.get(order.marketIndex);
				if (!params) {
					this.onGoingAuctions.delete(orderSignature);
					return;
				}

				const takerStats = await this.userStatsMap.mustGet(taker.authority.toString());
				const referrerInfo = takerStats.getReferrerInfo();

				console.log(`Trying to fill ${orderSignature}`);
				try {
					const { txSig } = await this.jitProxyClient.jit({
						takerKey,
						takerStatsKey,
						taker,
						takerOrderId: order.orderId,
						maxPosition: params.maxPosition,
						minPosition: params.minPosition,
						bid: params.bid,
						ask: params.ask,
						postOnly: null,
						priceType: params.priceType,
						referrerInfo,
						subAccountId: params.subAccountId,
					});

					console.log(`Filled ${orderSignature} txSig ${txSig}`);
					await sleep(10000);
					this.onGoingAuctions.delete(orderSignature);
					return;
				} catch (e) {
					console.error(`Failed to fill ${orderSignature}`);
					if (e.message.includes('0x1770') || e.message.includes('0x1771')) {
						console.log('Order does not cross params yet, retrying');
					} else if (e.message.includes('0x1793')) {
						console.log('Oracle invalid, retrying');
					} else {
						await sleep(10000);
						this.onGoingAuctions.delete(orderSignature);
						return;
					}
				}
				i++;
			}

			this.onGoingAuctions.delete(orderSignature);
		};
	}

	getOrderSignatures(takerKey: string, orderId: number): string {
		return `${takerKey}-${orderId}`;
	}

	public updatePerpParams(marketIndex: number, params: JitParams): void {
		this.perpParams.set(marketIndex, params);
	}

	public updateSpotParams(marketIndex: number, params: JitParams): void {
		this.spotParams.set(marketIndex, params);
	}

	public setUserFilter(userFilter: UserFilter | undefined): void {
		this.userFilter = userFilter;
	}
}

function sleep(ms: number): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, ms));
}
