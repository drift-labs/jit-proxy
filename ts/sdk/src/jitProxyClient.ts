import {
	BN,
	DriftClient,
	isVariant,
	PostOnlyParams,
	QUOTE_SPOT_MARKET_INDEX,
	ReferrerInfo,
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
	referrerInfo?: ReferrerInfo;
};

export class PriceType {
	static readonly LIMIT = { limit: {} };
	static readonly ORACLE = { oracle: {} };
}

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
}
