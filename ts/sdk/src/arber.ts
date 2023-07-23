import {
	BN,
	DLOBSource,
	DriftClient,
	getUserStatsAccountPublicKey,
	isVariant,
	MakerInfo,
	MarketType,
	SlotSource,
	UserMap
} from "@drift-labs/sdk";
import {JitProxyClient, PriceType} from "./jitProxyClient";
import {JitParams} from "./jitter";

export type TakeParams = {
	bid: BN;
	ask: BN;
	minPosition: BN;
	maxPosition: BN;
	priceType: PriceType;
};

export class Arber {
	driftClient: DriftClient;
	jitProxyClient: JitProxyClient;
	slotSource: SlotSource;
	userMap: UserMap;
	frequency: number;

	perpParams = new Map<number, TakeParams>();
	spotParams = new Map<number, TakeParams>();

	intervalId: NodeJS.Timeout | undefined;

	constructor({
		driftClient,
		jitProxyClient,
		slotSource,
		userMap,
		frequency = 1000,
				}: {
		driftClient: DriftClient;
		jitProxyClient: JitProxyClient;
		slotSource: SlotSource;
		userMap: UserMap;
		frequency?: number;
	}) {
		this.driftClient = driftClient;
		this.slotSource = slotSource;
		this.userMap = userMap;
		this.frequency = frequency;
	}

	public subscribe(): void {
		if (this.intervalId) {
			return;
		}

		this.intervalId = setInterval(this.tryArb.bind(this), this.frequency);
	}

	async tryArb() : Promise<void> {
		const slot = this.slotSource.getSlot();
		for (const [marketIndex, params] of this.perpParams) {
			const dlob = await this.userMap.getDLOB(slot);
			const oraclePriceData = this.driftClient.getOracleDataForPerpMarket(marketIndex);
			const restingBids = dlob.getRestingLimitBids(marketIndex, slot, MarketType.PERP, oraclePriceData);

			const takerAsk = this.getPriceFromParams(params, 'ask', oraclePriceData.price);
			for (const restingBid of restingBids) {
				const makerBid = restingBid.getPrice(oraclePriceData, slot);
				if (takerAsk.lte(makerBid)) {
					const makerUser = this.userMap.get(restingBid.userAccount.toString());
					if (!makerUser) {
						continue;
					}
					const makerInfo : MakerInfo = {
						maker: restingBid.userAccount,
						makerUserAccount: makerUser.getUserAccount(),
						makerStats: getUserStatsAccountPublicKey(this.driftClient.program.programId, makerUser.getUserAccount().authority),
					}
				} else {
					break
				}
			}
		}
	}

	public unsubscribe(): void {
		if (this.intervalId) {
			clearInterval(this.intervalId);
			this.intervalId = undefined;
		}
	}

	public updatePerpParams(marketIndex: number, params: TakeParams): void {
		this.perpParams.set(marketIndex, params);
	}

	public updateSpotParams(marketIndex: number, params: TakeParams): void {
		this.spotParams.set(marketIndex, params);
	}

	getPriceFromParams(params: TakeParams, side: 'bid' | 'ask', oraclePrice: BN) : BN {
		if (side === 'bid') {
			if (isVariant(params.priceType, 'oracle')) {
				return oraclePrice.add(params.bid);
			}
			return params.bid;
		} else {
			if (isVariant(params.priceType, 'oracle')) {
				return oraclePrice.add(params.ask);
			}
			return params.ask;
		}
	}
}