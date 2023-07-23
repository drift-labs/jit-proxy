import {
	BN,
	DriftClient,
	isVariant,
	MakerInfo,
	MarketType,
	PostOnlyParams,
	QUOTE_SPOT_MARKET_INDEX,
	TxParams,
	UserAccount,
} from '@drift-labs/sdk';
import { IDL, JitProxy } from './types/jit_proxy';
import { PublicKey, TransactionInstruction } from '@solana/web3.js';
import { Program } from '@coral-xyz/anchor';
import { TxSigAndSlot } from '@drift-labs/sdk/lib/tx/types';

export type JitIxParams = {
	takerKey: PublicKey;
	takerStatsKey: PublicKey;
	taker: UserAccount;
	takerOrderId: number;
	maxPosition: BN;
	minPosition: BN;
	bid: BN;
	ask: BN;
	postOnly: PostOnlyParams | null;
	priceType?: PriceType;
};

export class PriceType {
	static readonly LIMIT = { limit: {} };
	static readonly ORACLE = { oracle: {} };
}

export type TakeIxParams = {
	makerInfos: MakerInfo[];
	marketIndex: number;
	marketType: MarketType;
	maxPosition: BN;
	minPosition: BN;
	bid: BN;
	ask: BN;
	priceType?: PriceType;
	fulfillmentMethod: null;
};

export class JitProxyClient {
	private driftClient: DriftClient;
	private program: Program<JitProxy>;

	constructor({
		driftClient,
		programId,
	}: {
		driftClient: DriftClient;
		programId: PublicKey;
	}) {
		this.driftClient = driftClient;
		this.program = new Program(IDL, programId, driftClient.provider);
	}

	public async jit(
		params: JitIxParams,
		txParams?: TxParams
	): Promise<TxSigAndSlot> {
		const ix = await this.getJitIx(params);
		const tx = await this.driftClient.buildTransaction([ix], txParams);
		return await this.driftClient.sendTransaction(tx);
	}

	public async getJitIx({
		takerKey,
		takerStatsKey,
		taker,
		takerOrderId,
		maxPosition,
		minPosition,
		bid,
		ask,
		postOnly = null,
		priceType = PriceType.LIMIT,
	}: JitIxParams): Promise<TransactionInstruction> {
		const order = taker.orders.find((order) => order.orderId === takerOrderId);
		const remainingAccounts = this.driftClient.getRemainingAccounts({
			userAccounts: [taker, this.driftClient.getUserAccount()],
			writableSpotMarketIndexes: isVariant(order.marketType, 'spot')
				? [order.marketIndex, QUOTE_SPOT_MARKET_INDEX]
				: [],
			writablePerpMarketIndexes: isVariant(order.marketType, 'perp')
				? [order.marketIndex]
				: [],
		});

		if (isVariant(order.marketType, 'spot')) {
			remainingAccounts.push({
				pubkey: this.driftClient.getSpotMarketAccount(order.marketIndex).vault,
				isWritable: false,
				isSigner: false,
			});
			remainingAccounts.push({
				pubkey: this.driftClient.getQuoteSpotMarketAccount().vault,
				isWritable: false,
				isSigner: false,
			});
		}

		const jitParams = {
			takerOrderId,
			maxPosition,
			minPosition,
			bid,
			ask,
			postOnly,
			priceType,
		};

		return this.program.methods
			.jit(jitParams)
			.accounts({
				taker: takerKey,
				takerStats: takerStatsKey,
				state: await this.driftClient.getStatePublicKey(),
				user: await this.driftClient.getUserAccountPublicKey(),
				userStats: this.driftClient.getUserStatsAccountPublicKey(),
				driftProgram: this.driftClient.program.programId,
			})
			.remainingAccounts(remainingAccounts)
			.instruction();
	}

	public async take(
		params: TakeIxParams,
		txParams?: TxParams
	): Promise<TxSigAndSlot> {
		const ix = await this.getTakeIx(params);
		const tx = await this.driftClient.buildTransaction([ix], txParams);
		return await this.driftClient.sendTransaction(tx);
	}

	public async getTakeIx({
		makerInfos,
		marketIndex,
		marketType,
		maxPosition,
		minPosition,
		bid,
		ask,
		priceType = PriceType.LIMIT,
	}: TakeIxParams): Promise<TransactionInstruction> {
		const userAccounts = [this.driftClient.getUserAccount()];
		for (const makerInfo of makerInfos) {
			userAccounts.push(makerInfo.makerUserAccount);
		}

		const remainingAccounts = this.driftClient.getRemainingAccounts({
			userAccounts,
			writableSpotMarketIndexes: isVariant(marketType, 'spot')
				? [marketIndex, QUOTE_SPOT_MARKET_INDEX]
				: [],
			writablePerpMarketIndexes: isVariant(marketType, 'perp')
				? [marketIndex]
				: [],
		});

		for (const makerInfo of makerInfos) {
			remainingAccounts.push({
				pubkey: makerInfo.maker,
				isWritable: true,
				isSigner: false,
			});
			remainingAccounts.push({
				pubkey: makerInfo.makerStats,
				isWritable: true,
				isSigner: false,
			});
		}

		if (isVariant(marketType, 'spot')) {
			remainingAccounts.push({
				pubkey: this.driftClient.getSpotMarketAccount(marketIndex).vault,
				isWritable: false,
				isSigner: false,
			});
			remainingAccounts.push({
				pubkey: this.driftClient.getQuoteSpotMarketAccount().vault,
				isWritable: false,
				isSigner: false,
			});
		}

		const takeParams = {
			marketIndex,
			marketType,
			maxPosition,
			minPosition,
			bid,
			ask,
			priceType,
			fulfillmentMethod: null,
		};

		return this.program.methods
			.take(takeParams)
			.accounts({
				state: await this.driftClient.getStatePublicKey(),
				user: await this.driftClient.getUserAccountPublicKey(),
				userStats: this.driftClient.getUserStatsAccountPublicKey(),
				driftProgram: this.driftClient.program.programId,
			})
			.remainingAccounts(remainingAccounts)
			.instruction();
	}
}
