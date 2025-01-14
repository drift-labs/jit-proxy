import { JitProxyClient } from '../jitProxyClient';
import { PublicKey } from '@solana/web3.js';
import {
	AuctionSubscriber,
	DriftClient,
	Order,
	PostOnlyParams,
	SwiftOrderSubscriber,
	UserAccount,
	UserStatsMap,
} from '@drift-labs/sdk';
import { BaseJitter } from './baseJitter';
import { SignedSwiftOrderParams } from '@drift-labs/sdk/lib/node/swift/types';

export class JitterShotgun extends BaseJitter {
	constructor({
		auctionSubscriber,
		jitProxyClient,
		driftClient,
		userStatsMap,
		swiftOrderSubscriber,
	}: {
		driftClient: DriftClient;
		auctionSubscriber: AuctionSubscriber;
		jitProxyClient: JitProxyClient;
		userStatsMap?: UserStatsMap;
		swiftOrderSubscriber?: SwiftOrderSubscriber;
	}) {
		super({
			auctionSubscriber,
			jitProxyClient,
			driftClient,
			userStatsMap,
			swiftOrderSubscriber,
		});
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

			const takerStats = await this.userStatsMap.mustGet(
				taker.authority.toString()
			);
			const referrerInfo = takerStats.getReferrerInfo();

			// assumes each preflight simulation takes ~1 slot
			while (i < order.auctionDuration) {
				const params = this.perpParams.get(order.marketIndex);
				if (!params) {
					this.deleteOnGoingAuction(orderSignature);
					return;
				}

				const txParams = {
					computeUnits: this.computeUnits,
					computeUnitsPrice: this.computeUnitsPrice,
				};

				console.log(`Trying to fill ${orderSignature}`);
				try {
					const { txSig } = await this.jitProxyClient.jit(
						{
							takerKey,
							takerStatsKey,
							taker,
							takerOrderId: order.orderId,
							maxPosition: params.maxPosition,
							minPosition: params.minPosition,
							bid: params.bid,
							ask: params.ask,
							postOnly: params.postOnlyParams ?? PostOnlyParams.MUST_POST_ONLY,
							priceType: params.priceType,
							referrerInfo,
							subAccountId: params.subAccountId,
						},
						txParams
					);

					console.log(
						`Successfully sent tx for ${orderSignature} txSig ${txSig}`
					);
					await sleep(10000);
					this.deleteOnGoingAuction(orderSignature);
					return;
				} catch (e) {
					console.error(`Failed to fill ${orderSignature}`);
					if (e.message.includes('0x1770') || e.message.includes('0x1771')) {
						console.log('Order does not cross params yet, retrying');
					} else if (e.message.includes('0x1779')) {
						console.log('Order could not fill');
					} else if (e.message.includes('0x1793')) {
						console.log('Oracle invalid, retrying');
					} else {
						await sleep(10000);
						this.deleteOnGoingAuction(orderSignature);
						return;
					}
				}
				i++;
			}

			this.deleteOnGoingAuction(orderSignature);
		};
	}

	createTrySwiftFill(
		authorityToUse: PublicKey,
		signedSwiftOrderParams: SignedSwiftOrderParams,
		uuid: Uint8Array,
		taker: UserAccount,
		takerKey: PublicKey,
		takerStatsKey: PublicKey,
		order: Order,
		orderSignature: string,
		marketIndex: number
	): () => Promise<void> {
		return async () => {
			let i = 0;

			const takerStats = await this.userStatsMap.mustGet(
				taker.authority.toString()
			);
			const referrerInfo = takerStats.getReferrerInfo();

			// assumes each preflight simulation takes ~1 slot
			while (i < order.auctionDuration) {
				const params = this.perpParams.get(order.marketIndex);
				if (!params) {
					this.deleteOnGoingAuction(orderSignature);
					return;
				}

				const txParams = {
					computeUnits: this.computeUnits,
					computeUnitsPrice: this.computeUnitsPrice,
				};

				console.log(`Trying to fill ${orderSignature}`);
				try {
					const { txSig } = await this.jitProxyClient.jitSwift(
						{
							takerKey,
							takerStatsKey,
							taker,
							takerOrderId: order.orderId,
							maxPosition: params.maxPosition,
							minPosition: params.minPosition,
							bid: params.bid,
							ask: params.ask,
							postOnly: params.postOnlyParams ?? PostOnlyParams.MUST_POST_ONLY,
							priceType: params.priceType,
							referrerInfo,
							subAccountId: params.subAccountId,
							authorityToUse,
							signedSwiftOrderParams,
							uuid,
							marketIndex,
						},
						txParams
					);

					console.log(
						`Successfully sent tx for ${orderSignature} txSig ${txSig}`
					);
					await sleep(10000);
					this.deleteOnGoingAuction(orderSignature);
					return;
				} catch (e) {
					console.error(`Failed to fill ${orderSignature}`);
					if (e.message.includes('0x1770') || e.message.includes('0x1771')) {
						console.log('Order does not cross params yet, retrying');
					} else if (e.message.includes('0x1779')) {
						console.log('Order could not fill');
					} else if (e.message.includes('0x1793')) {
						console.log('Oracle invalid, retrying');
					} else {
						await sleep(10000);
						this.deleteOnGoingAuction(orderSignature);
						return;
					}
				}
				i++;
			}

			this.deleteOnGoingAuction(orderSignature);
		};
	}
}

function sleep(ms: number): Promise<void> {
	return new Promise((resolve) => setTimeout(resolve, ms));
}
