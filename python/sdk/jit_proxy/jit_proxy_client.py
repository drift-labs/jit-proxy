from dataclasses import dataclass
from typing import Optional, cast

from solders.pubkey import Pubkey

from driftpy.types import UserAccount, PostOnlyParams, ReferrerInfo, MarketType, MakerInfo, is_variant
from driftpy.drift_client import DriftClient
from driftpy.constants.numeric_constants import QUOTE_SPOT_MARKET_INDEX

from anchorpy import Context, Program
from solana.transaction import AccountMeta

from jit_proxy.jit_client.types import PriceTypeKind

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
    price_type: Optional[PriceTypeKind]
    referrer_info: Optional[ReferrerInfo]
    sub_account_id: Optional[int]

@dataclass
class ArbIxParams:
    maker_infos: list[MakerInfo]
    market_index: int

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
        self.program = await Program.at(self.program_id, self.drift_client.program.provider)

    async def jit(self, params: JitIxParams):
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

        if str(params.price_type) == 'Oracle()':
            price_type = self.program.type['PriceType'].Oracle()
        elif str(params.price_type) == 'Limit()':
            price_type = self.program.type['PriceType'].Limit()
        else:
            raise ValueError(f"Unknown price type: {params.price_type}")

        jit_params = self.program.type["JitParams"](
            taker_order_id=params.taker_order_id,
            max_position=cast(int, params.max_position),
            min_position=cast(int, params.min_position),
            bid=cast(int, params.bid),
            ask=cast(int, params.ask),
            price_type=price_type,
            post_only=params.post_only
        )

        ix = self.program.instruction['jit'](
            jit_params, 
            ctx = Context(
                    accounts = {
                        "state" : self.drift_client.get_state_public_key(),
                        "user" : self.drift_client.get_user_account_public_key(sub_account_id),
                        "user_stats" : self.drift_client.get_user_stats_public_key(),
                        "taker" : params.taker_key,
                        "taker_stats" : params.taker_stats_key,
                        "authority" : self.drift_client.wallet.public_key,
                        "drift_program" : self.drift_client.program_id
                    },
                    signers = {self.drift_client.wallet},
                    remaining_accounts = remaining_accounts
                ),
        )    

        tx_sig_and_slot = await self.drift_client.send_ixs(ix)

        return tx_sig_and_slot.tx_sig     







