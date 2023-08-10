import { JitProxyClient, PriceType } from './jitProxyClient';
import { PublicKey } from '@solana/web3.js';
import {
	AuctionSubscriber,
	BN,
	BulkAccountLoader,
	convertToNumber,
	DriftClient,
	getAuctionPrice,
	getAuctionPriceForOracleOffsetAuction,
	getUserStatsAccountPublicKey,
	hasAuctionPrice,
	isVariant,
	OraclePriceData,
	Order,
	PRICE_PRECISION,
	SlotSubscriber,
	UserAccount,
	UserStatsMap,
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

type AuctionAndOrderDetails = {
	slotsTilCross: number;
	willCross: boolean;
	bid: number;
	ask: number;
	auctionStartPrice: number;
	auctionEndPrice: number;
	stepSize: number;
	oraclePrice: OraclePriceData;
};

export class Jitter {
	auctionSubscriber: AuctionSubscriber;
	slotSubscriber: SlotSubscriber;
	driftClient: DriftClient;
	jitProxyClient: JitProxyClient;
	userStatsMap: UserStatsMap;

	perpParams = new Map<number, JitParams>();
	spotParams = new Map<number, JitParams>();

	onGoingAuctions = new Map<string, Promise<void>>();

	userFilter: UserFilter;

	constructor({
		auctionSubscriber,
		slotSubscriber,
		jitProxyClient,
		driftClient,
		userStatsMap,
	}: {
		driftClient: DriftClient;
		slotSubscriber: SlotSubscriber;
		auctionSubscriber: AuctionSubscriber;
		jitProxyClient: JitProxyClient;
		userStatsMap?: UserStatsMap;
	}) {
		this.auctionSubscriber = auctionSubscriber;
		this.slotSubscriber = slotSubscriber;
		this.driftClient = driftClient;
		this.jitProxyClient = jitProxyClient;
		this.userStatsMap =
			userStatsMap ||
			new UserStatsMap(this.driftClient, {
				type: 'polling',
				accountLoader: new BulkAccountLoader(
					this.driftClient.connection,
					'confirmed',
					0
				),
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

						const perpMarketAccount = this.driftClient.getPerpMarketAccount(
							order.marketIndex
						);
						if (
							order.baseAssetAmount
								.sub(order.baseAssetAmountFilled)
								.lte(perpMarketAccount.amm.minOrderSize)
						) {
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

						const spotMarketAccount = this.driftClient.getSpotMarketAccount(
							order.marketIndex
						);
						if (
							order.baseAssetAmount
								.sub(order.baseAssetAmountFilled)
								.lte(spotMarketAccount.minOrderSize)
						) {
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
			const params = this.perpParams.get(order.marketIndex);
			if (!params) {
				this.onGoingAuctions.delete(orderSignature);
				return;
			}

			const takerStats = await this.userStatsMap.mustGet(
				taker.authority.toString()
			);
			const referrerInfo = takerStats.getReferrerInfo();

			const {
				slotsTilCross,
				willCross,
				bid,
				ask,
				auctionStartPrice,
				auctionEndPrice,
				stepSize,
				oraclePrice,
			} = this.getAuctionAndOrderDetails(order);

			console.log(`
				Taker wants to ${JSON.stringify(
					order.direction
				)}, order slot is ${order.slot.toNumber()},
				My market: ${bid}@${ask},
				Auction: ${auctionStartPrice} -> ${auctionEndPrice}, step size ${stepSize}
				Current slot: ${
					this.slotSubscriber.currentSlot
				}, Order slot: ${order.slot.toNumber()},
				Will cross?: ${willCross}
				Slots to wait: ${slotsTilCross}. Target slot = ${
				order.slot.toNumber() + slotsTilCross
			}
			`);

			this.waitForSlotOrCrossOrExpiry(
				willCross
					? order.slot.toNumber() + slotsTilCross
					: order.slot.toNumber() + order.auctionDuration + 1,
				order,
				{
					slotsTilCross,
					willCross,
					bid,
					ask,
					auctionStartPrice,
					auctionEndPrice,
					stepSize,
					oraclePrice,
				}
			).then(async ({ slot, updatedDetails }) => {
				if (slot === -1) {
					console.log('Auction expired without crossing');
					this.onGoingAuctions.delete(orderSignature);
					return;
				}

				const params = isVariant(order.marketType, 'perp')
					? this.perpParams.get(order.marketIndex)
					: this.spotParams.get(order.marketIndex);
				const bid = isVariant(params.priceType, 'oracle')
					? convertToNumber(oraclePrice.price.sub(params.bid), PRICE_PRECISION)
					: convertToNumber(params.bid, PRICE_PRECISION);
				const ask = isVariant(params.priceType, 'oracle')
					? convertToNumber(oraclePrice.price.add(params.ask), PRICE_PRECISION)
					: convertToNumber(params.ask, PRICE_PRECISION);
				const auctionPrice = convertToNumber(
					getAuctionPrice(order, slot, updatedDetails.oraclePrice.price),
					PRICE_PRECISION
				);
				console.log(`
					Expected auction price: ${auctionStartPrice + slotsTilCross * stepSize}
					Actual auction price: ${auctionPrice}
					-----------------
					Looking for slot ${order.slot.toNumber() + slotsTilCross}
					Got slot ${slot}
				`);

				console.log(`Trying to fill ${orderSignature} with:
					market: ${bid}@${ask}
					auction price: ${auctionPrice}
					submitting" ${convertToNumber(params.bid, PRICE_PRECISION)}@${convertToNumber(
					params.ask,
					PRICE_PRECISION
				)}
				`);
				let i = 0;
				while (i < 3) {
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
						await sleep(3000);
						this.onGoingAuctions.delete(orderSignature);
						return;
					} catch (e) {
						console.error(`Failed to fill ${orderSignature}`);
						if (e.message.includes('0x1770') || e.message.includes('0x1771')) {
							console.log('Order does not cross params yet');
						} else if (e.message.includes('0x1793')) {
							console.log('Oracle invalid');
						} else {
							await sleep(3000);
							this.onGoingAuctions.delete(orderSignature);
							return;
						}
					}
					await sleep(50);
					i++;
				}
			});
			this.onGoingAuctions.delete(orderSignature);
		};
	}

	getAuctionAndOrderDetails(order: Order): AuctionAndOrderDetails {
		// Find number of slots until the order is expected to be in cross
		const params = isVariant(order.marketType, 'perp')
			? this.perpParams.get(order.marketIndex)
			: this.spotParams.get(order.marketIndex);
		const oraclePrice = isVariant(order.marketType, 'perp')
			? this.driftClient.getOracleDataForPerpMarket(order.marketIndex)
			: this.driftClient.getOracleDataForSpotMarket(order.marketIndex);

		const makerOrderDir = isVariant(order.direction, 'long') ? 'sell' : 'buy';
		const auctionStartPrice = convertToNumber(
			isVariant(order.orderType, 'oracle')
				? getAuctionPriceForOracleOffsetAuction(
						order,
						order.slot.toNumber(),
						oraclePrice.price
				  )
				: order.auctionStartPrice,
			PRICE_PRECISION
		);
		const auctionEndPrice = convertToNumber(
			isVariant(order.orderType, 'oracle')
				? getAuctionPriceForOracleOffsetAuction(
						order,
						order.slot.toNumber() + order.auctionDuration - 1,
						oraclePrice.price
				  )
				: order.auctionEndPrice,
			PRICE_PRECISION
		);

		const bid = isVariant(params.priceType, 'oracle')
			? convertToNumber(oraclePrice.price.sub(params.bid), PRICE_PRECISION)
			: convertToNumber(params.bid, PRICE_PRECISION);
		const ask = isVariant(params.priceType, 'oracle')
			? convertToNumber(oraclePrice.price.add(params.ask), PRICE_PRECISION)
			: convertToNumber(params.ask, PRICE_PRECISION);

		let slotsTilCross = 0;
		let willCross = false;
		const stepSize =
			(auctionEndPrice - auctionStartPrice) / (order.auctionDuration - 1);
		while (slotsTilCross < order.auctionDuration) {
			if (makerOrderDir === 'buy') {
				if (
					convertToNumber(
						getAuctionPrice(
							order,
							order.slot.toNumber() + slotsTilCross,
							oraclePrice.price
						),
						PRICE_PRECISION
					) <= bid
				) {
					willCross = true;
					break;
				}
			} else {
				if (
					convertToNumber(
						getAuctionPrice(
							order,
							order.slot.toNumber() + slotsTilCross,
							oraclePrice.price
						),
						PRICE_PRECISION
					) >= ask
				) {
					willCross = true;
					break;
				}
			}
			slotsTilCross++;
		}

		return {
			slotsTilCross,
			willCross,
			bid,
			ask,
			auctionStartPrice,
			auctionEndPrice,
			stepSize,
			oraclePrice,
		};
	}

	async waitForSlotOrCrossOrExpiry(
		targetSlot: number,
		order: Order,
		initialDetails: AuctionAndOrderDetails
	): Promise<{ slot: number; updatedDetails: AuctionAndOrderDetails }> {
		const auctionEndSlot = order.auctionDuration + order.slot.toNumber();
		let currentDetails: AuctionAndOrderDetails = initialDetails;
		let willCross = initialDetails.willCross;
		return new Promise((resolve) => {
			// Immediately return if we are past target slot

			// Otherwise listen for new slots in case we hit the target slot and we're gonna cross
			this.slotSubscriber.eventEmitter.on('newSlot', (slot) => {
				if (slot >= targetSlot && willCross) {
					resolve({ slot, updatedDetails: currentDetails });
				}
			});

			// Update target slot as the bid/ask and the oracle changes
			setInterval(async () => {
				if (this.slotSubscriber.currentSlot >= auctionEndSlot) {
					resolve({ slot: -1, updatedDetails: currentDetails });
				}

				currentDetails = this.getAuctionAndOrderDetails(order);
				willCross = currentDetails.willCross;
				if (willCross) {
					targetSlot = order.slot.toNumber() + currentDetails.slotsTilCross;
				}
			}, 100);
		});
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
