from dataclasses import asdict, dataclass
from typing import Optional
from solders.pubkey import Pubkey
from driftpy.types import UserAccount, PostOnlyParams, ReferrerInfo, MarketType, MakerInfo, is_variant
from driftpy.drift_client import DriftClient
from driftpy.constants.numeric_constants import QUOTE_SPOT_MARKET_INDEX
from borsh_construct.enum import _rust_enum
from sumtypes import constructor
from anchorpy import Program
from solana.transaction import AccountMeta, Instruction
from jit_proxy.jit_client.instructions import jit, check_order_constraints, arb_perp

@_rust_enum
class PriceType:
    Limit: constructor()
    Oracle: constructor()

@dataclass
class JitIxParams:
    taker_key: Pubkey
    taker_stats_key: Pubkey
    taker: UserAccount
    taker_order_id: int
    max_position: int
    min_position: int
    bid: int
    ask: int
    post_only: Optional[PostOnlyParams]
    price_type: Optional[PriceType]
    referrer_info: Optional[ReferrerInfo]
    sub_account_id: Optional[int]

@dataclass
class ArbIxParams:
    maker_infos: list[MakerInfo]
    market_index: int

@dataclass
class JitParams:
    taker_order_id: int
    max_position: int
    min_position: int
    bid: int
    ask: int
    price_type: PriceType
    post_only: Optional[PostOnlyParams]

@dataclass
class OrderConstraint:
    max_position: int
    min_position: int
    market_index: int
    market_type: MarketType

class JitProxyClient:
    def __init__(self, drift_client: DriftClient, program_id: Pubkey):
        self.program_id = program_id
        self.drift_client = drift_client
        self.program = None

    async def init(self):
        idl = await Program.fetch_idl(self.program_id, self.drift_client.program.provider)
        self.program = Program(idl, self.program_id)

    async def jit(self, params: JitIxParams):
        if self.program is None:
            await self.init()
        ix = await self.get_jit_ix(params)
        return await self.drift_client.send_ixs([ix], self.drift_client.wallet)
    
    async def get_jit_ix(self, params: JitIxParams) -> Instruction:
        if self.program is None:
            await self.init()
        sub_account_id = params.sub_account_id if params.sub_account_id is not None else self.drift_client.active_sub_account_id
        order = next((order for order in params.taker.orders if order.order_id == params.taker_order_id), None)
        remaining_accounts = self.drift_client.get_remaining_accounts(
            user_accounts = [params.taker, self.drift_client.get_user_account(sub_account_id)],
            writable_spot_market_indexes = [order.market_index, QUOTE_SPOT_MARKET_INDEX] \
                if is_variant(order.market_type, 'Spot') else [],
            writable_perp_market_indexes = [order.market_index] \
                if is_variant(order.market_type, 'Perp') else [],
         )
        
        if is_variant(order.market_type, 'Spot'):
            remaining_accounts.append(AccountMeta(
                pubkey = self.drift_client.get_spot_market_account(order.market_index).vault,
                is_writable = False,
                is_signer = False
            ))
            remaining_accounts.append(AccountMeta(
                pubkey = self.drift_client.get_quote_spot_market_account().vault,
                is_writable = False,
                is_signer = False
            ))

        jit_params = JitParams(
            taker_order_id=params.taker_order_id,
            max_position=params.max_position,
            min_position=params.min_position,
            bid=params.bid,
            ask=params.ask,
            price_type=params.price_type,
            post_only=params.post_only
        )

        return jit(
            {
                "params" : jit_params
            },
            {
                "state" : self.drift_client.get_state_account(),
                "user" : self.drift_client.get_user_account_public_key(sub_account_id),
                "user_stats" : self.drift_client.get_user_stats_public_key(),
                "taker" : params.taker_key,
                "taker_stats" : params.taker_stats_key,
                "authority" : self.drift_client.wallet.public_key,
                "drift_program" : self.drift_client.program_id
            },
            self.drift_client.program_id,
            remaining_accounts
        )

    async def get_check_order_constraint_ix(self, sub_account_id: int, order_constraints: list[OrderConstraint]) -> Instruction:
        if self.program is None:
            await self.init()
        sub_account_id = sub_account_id if sub_account_id is not None else self.drift_client.active_sub_account_id

        readable_perp_market_indexes = []
        readable_spot_market_indexes = []

        for constraint in order_constraints:
            if is_variant(constraint.market_type, 'Perp'):
                readable_perp_market_indexes.append(constraint.market_index)
            else:
                readable_spot_market_indexes.append(constraint.market_index)

        remaining_accounts = self.drift_client.get_remaining_accounts(
            [self.drift_client.get_user_account(sub_account_id)],
            readable_spot_market_indexes = readable_spot_market_indexes,
            readable_perp_market_indexes = readable_perp_market_indexes
        )

        return check_order_constraints(
            {
                "constraints": order_constraints
            },
            {
                "user" : await self.drift_client.get_user_account_public_key(sub_account_id)
            },
            self.drift_client.program_id,
            remaining_accounts
        )

    async def arb_perp(self, params: ArbIxParams):
        if self.program is None:
            await self.init()
        ix = await self.get_arb_perp_ix(params.maker_infos, params.market_index)
        return await self.drift_client.send_ixs([ix], self.drift_client.wallet)

    async def get_arb_perp_ix(self, maker_infos: list[MakerInfo], market_index: int, referrer_info: Optional[ReferrerInfo]) -> Instruction:
        if self.program is None:
            await self.init()
        user_accounts = [self.drift_client.get_user_account()]
        for maker_info in maker_infos:
            user_accounts.append(maker_info.maker_user_account)

        remaining_accounts = self.drift_client.get_remaining_accounts(
            user_accounts,
            writable_perp_market_indexes = [market_index]
        )

        for maker_info in maker_infos:
            remaining_accounts.append(AccountMeta(
                pubkey = maker_info.maker,
                is_writable = True,
                is_signer = False
            ))
            remaining_accounts.append(AccountMeta(
                pubkey = maker_info.maker_stats,
                is_writable = True,
                is_signer = False
            ))

        if referrer_info is not None:
            is_referrer_maker = next((maker for maker in maker_infos if maker.maker == referrer_info.referrer), None)
            if not is_referrer_maker:
                remaining_accounts.append(AccountMeta(
                    pubkey = referrer_info.referrer,
                    is_writable = True,
                    is_signer = False
                ))
                remaining_accounts.append(AccountMeta(
                    pubkey = referrer_info.referrer_stats,
                    is_writable = True,
                    is_signer = False
                ))

        return arb_perp(
            {
                "market_index" : market_index
            },
            {
                "state" : await self.drift_client.get_state_public_key(),
                "user" : await self.drift_client.get_user_account_public_key(),
                "user_stats" : await self.drift_client.get_user_stats_public_key(),
                "authority" : self.drift_client.wallet.public_key,
                "drift_program" : self.drift_client.program_id
            },
            self.drift_client.program_id,
            remaining_accounts
        )







