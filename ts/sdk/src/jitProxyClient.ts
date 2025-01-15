import {
	BN,
	DriftClient,
	getSwiftUserAccountPublicKey,
	isVariant,
	MakerInfo,
	MarketType,
	PostOnlyParams,
	QUOTE_SPOT_MARKET_INDEX,
	ReferrerInfo,
	TxParams,
	UserAccount,
} from '@drift-labs/sdk';
import { IDL, JitProxy } from './types/jit_proxy';
import {
	PublicKey,
	SYSVAR_INSTRUCTIONS_PUBKEY,
	TransactionInstruction,
	TransactionMessage,
	VersionedTransaction,
} from '@solana/web3.js';
import { Program } from '@coral-xyz/anchor';
import { TxSigAndSlot } from '@drift-labs/sdk';
import { SignedSwiftOrderParams } from '@drift-labs/sdk/lib/node/swift/types';

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
	subAccountId?: number;
};

export type JitSwiftIxParams = JitIxParams & {
	authorityToUse: PublicKey;
	signedSwiftOrderParams: SignedSwiftOrderParams;
	uuid: Uint8Array;
	marketIndex: number;
};

export class PriceType {
	static readonly LIMIT = { limit: {} };
	static readonly ORACLE = { oracle: {} };
}

export type OrderConstraint = {
	maxPosition: BN;
	minPosition: BN;
	marketIndex: number;
	marketType: MarketType;
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

	public async jitSwift(
		params: JitSwiftIxParams,
		txParams?: TxParams,
		precedingIxs?: TransactionInstruction[]
	): Promise<TxSigAndSlot> {
		const swiftTakerIxs = await this.driftClient.getPlaceSwiftTakerPerpOrderIxs(
			params.signedSwiftOrderParams,
			params.marketIndex,
			{
				taker: params.takerKey,
				takerStats: params.takerStatsKey,
				takerUserAccount: params.taker,
			},
			params.authorityToUse,
			precedingIxs
		);

		const ix = await this.getJitSwiftIx(params);
		const tx = await this.driftClient.buildTransaction(
			[...swiftTakerIxs, ix],
			txParams
		);
		let resp;
		try {
			const message = new TransactionMessage({
				payerKey: this.driftClient.wallet.payer.publicKey,
				recentBlockhash: (
					await this.driftClient.connection.getLatestBlockhash()
				).blockhash,
				instructions: [...swiftTakerIxs, ix],
			}).compileToV0Message([this.driftClient.lookupTableAccount]);

			const tx = new VersionedTransaction(message);
			resp = await this.driftClient.connection.simulateTransaction(tx, {
				sigVerify: false,
				replaceRecentBlockhash: true,
				commitment: 'processed',
			});
			console.log(resp);
		} catch (e) {
			console.error(e);
		}

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
		referrerInfo,
		subAccountId,
	}: JitIxParams): Promise<TransactionInstruction> {
		subAccountId =
			subAccountId !== undefined
				? subAccountId
				: this.driftClient.activeSubAccountId;
		const order = taker.orders.find((order) => order.orderId === takerOrderId);
		const remainingAccounts = this.driftClient.getRemainingAccounts({
			userAccounts: [taker, this.driftClient.getUserAccount(subAccountId)],
			writableSpotMarketIndexes: isVariant(order.marketType, 'spot')
				? [order.marketIndex, QUOTE_SPOT_MARKET_INDEX]
				: [],
			writablePerpMarketIndexes: isVariant(order.marketType, 'perp')
				? [order.marketIndex]
				: [],
		});

		if (referrerInfo) {
			remainingAccounts.push({
				pubkey: referrerInfo.referrer,
				isWritable: true,
				isSigner: false,
			});
			remainingAccounts.push({
				pubkey: referrerInfo.referrerStats,
				isWritable: true,
				isSigner: false,
			});
		}

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
				user: await this.driftClient.getUserAccountPublicKey(subAccountId),
				userStats: this.driftClient.getUserStatsAccountPublicKey(),
				driftProgram: this.driftClient.program.programId,
			})
			.remainingAccounts(remainingAccounts)
			.instruction();
	}

	public async getJitSwiftIx({
		takerKey,
		takerStatsKey,
		taker,
		maxPosition,
		minPosition,
		bid,
		ask,
		postOnly = null,
		priceType = PriceType.LIMIT,
		referrerInfo,
		subAccountId,
		uuid,
		marketIndex,
	}: JitSwiftIxParams): Promise<TransactionInstruction> {
		subAccountId =
			subAccountId !== undefined
				? subAccountId
				: this.driftClient.activeSubAccountId;
		const remainingAccounts = this.driftClient.getRemainingAccounts({
			userAccounts: [taker, this.driftClient.getUserAccount(subAccountId)],
			writableSpotMarketIndexes: [],
			writablePerpMarketIndexes: [marketIndex],
		});

		if (referrerInfo) {
			remainingAccounts.push({
				pubkey: referrerInfo.referrer,
				isWritable: true,
				isSigner: false,
			});
			remainingAccounts.push({
				pubkey: referrerInfo.referrerStats,
				isWritable: true,
				isSigner: false,
			});
		}

		const jitSwiftParams = {
			swiftOrderUuid: Array.from(uuid),
			maxPosition,
			minPosition,
			bid,
			ask,
			postOnly,
			priceType,
		};

		return this.program.methods
			.jitSwift(jitSwiftParams)
			.accounts({
				taker: takerKey,
				takerStats: takerStatsKey,
				takerSwiftUserOrders: getSwiftUserAccountPublicKey(
					this.driftClient.program.programId,
					takerKey
				),
				authority: this.driftClient.wallet.payer.publicKey,
				state: await this.driftClient.getStatePublicKey(),
				user: await this.driftClient.getUserAccountPublicKey(subAccountId),
				userStats: this.driftClient.getUserStatsAccountPublicKey(),
				driftProgram: this.driftClient.program.programId,
				ixSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
			})
			.remainingAccounts(remainingAccounts)
			.instruction();
	}

	public async getCheckOrderConstraintIx({
		subAccountId,
		orderConstraints,
	}: {
		subAccountId: number;
		orderConstraints: OrderConstraint[];
	}): Promise<TransactionInstruction> {
		subAccountId =
			subAccountId !== undefined
				? subAccountId
				: this.driftClient.activeSubAccountId;

		const readablePerpMarketIndex = [];
		const readableSpotMarketIndexes = [];
		for (const orderConstraint of orderConstraints) {
			if (isVariant(orderConstraint.marketType, 'perp')) {
				readablePerpMarketIndex.push(orderConstraint.marketIndex);
			} else {
				readableSpotMarketIndexes.push(orderConstraint.marketIndex);
			}
		}

		const remainingAccounts = this.driftClient.getRemainingAccounts({
			userAccounts: [this.driftClient.getUserAccount(subAccountId)],
			readableSpotMarketIndexes,
			readablePerpMarketIndex,
		});

		return this.program.methods
			.checkOrderConstraints(orderConstraints)
			.accounts({
				user: await this.driftClient.getUserAccountPublicKey(subAccountId),
			})
			.remainingAccounts(remainingAccounts)
			.instruction();
	}

	public async arbPerp(
		params: {
			makerInfos: MakerInfo[];
			marketIndex: number;
		},
		txParams?: TxParams
	): Promise<TxSigAndSlot> {
		const ix = await this.getArbPerpIx(params);
		const tx = await this.driftClient.buildTransaction([ix], txParams);
		return await this.driftClient.sendTransaction(tx);
	}

	public async getArbPerpIx({
		makerInfos,
		marketIndex,
		referrerInfo,
	}: {
		makerInfos: MakerInfo[];
		marketIndex: number;
		referrerInfo?: ReferrerInfo;
	}): Promise<TransactionInstruction> {
		const userAccounts = [this.driftClient.getUserAccount()];
		for (const makerInfo of makerInfos) {
			userAccounts.push(makerInfo.makerUserAccount);
		}

		const remainingAccounts = this.driftClient.getRemainingAccounts({
			userAccounts,
			writablePerpMarketIndexes: [marketIndex],
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

		if (referrerInfo) {
			const referrerIsMaker =
				makerInfos.find((maker) =>
					maker.maker.equals(referrerInfo.referrer)
				) !== undefined;
			if (!referrerIsMaker) {
				remainingAccounts.push({
					pubkey: referrerInfo.referrer,
					isWritable: true,
					isSigner: false,
				});
				remainingAccounts.push({
					pubkey: referrerInfo.referrerStats,
					isWritable: true,
					isSigner: false,
				});
			}
		}

		return this.program.methods
			.arbPerp(marketIndex)
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
