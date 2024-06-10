from dataclasses import dataclass
from typing import Optional, cast

from borsh_construct.enum import _rust_enum
from sumtypes import constructor # type: ignore

from solders.pubkey import Pubkey # type: ignore

from anchorpy import Context, Program

from solana.transaction import AccountMeta

from driftpy.types import (
    UserAccount,
    PostOnlyParams,
    ReferrerInfo,
    MarketType,
    MakerInfo,
    is_variant,
)
from driftpy.drift_client import DriftClient
from driftpy.constants.numeric_constants import QUOTE_SPOT_MARKET_INDEX


@_rust_enum
class PriceType:
    Limit = constructor()
    Oracle = constructor()


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
    price_type: Optional[PriceType]
    referrer_info: Optional[ReferrerInfo]
    sub_account_id: Optional[int]
    post_only: PostOnlyParams = PostOnlyParams.MustPostOnly()


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
        self.program = await Program.at(
            self.program_id, self.drift_client.program.provider
        )

    async def jit(self, params: JitIxParams):
        if self.program is None:
            await self.init()

        sub_account_id = self.drift_client.get_sub_account_id_for_ix(
            params.sub_account_id # type: ignore
        )

        order = next(
            (
                order
                for order in params.taker.orders
                if order.order_id == params.taker_order_id
            ),
            None,
        )
        remaining_accounts = self.drift_client.get_remaining_accounts(
            user_accounts=[
                params.taker,
                self.drift_client.get_user_account(sub_account_id),
            ],
            writable_spot_market_indexes=[order.market_index, QUOTE_SPOT_MARKET_INDEX] # type: ignore
            if is_variant(order.market_type, "Spot") # type: ignore
            else [],
            writable_perp_market_indexes=[order.market_index] # type: ignore
            if is_variant(order.market_type, "Perp") # type: ignore
            else [],
        )

        if params.referrer_info is not None:
            remaining_accounts.append(
                AccountMeta(
                    pubkey=params.referrer_info.referrer,
                    is_writable=True,
                    is_signer=False,
                )
            )
            remaining_accounts.append(
                AccountMeta(
                    pubkey=params.referrer_info.referrer_stats,
                    is_writable=True,
                    is_signer=False,
                )
            )

        if is_variant(order.market_type, "Spot"): # type: ignore
            remaining_accounts.append(
                AccountMeta(
                    pubkey=self.drift_client.get_spot_market_account( # type: ignore
                        order.market_index # type: ignore
                    ).vault,
                    is_writable=False,
                    is_signer=False,
                )
            )
            remaining_accounts.append(
                AccountMeta(
                    pubkey=self.drift_client.get_quote_spot_market_account().vault, # type: ignore
                    is_writable=False,
                    is_signer=False,
                )
            )

        jit_params = self.program.type["JitParams"]( # type: ignore
            taker_order_id=params.taker_order_id,
            max_position=cast(int, params.max_position),
            min_position=cast(int, params.min_position),
            bid=cast(int, params.bid),
            ask=cast(int, params.ask),
            price_type=self.get_price_type(params.price_type), # type: ignore
            post_only=self.get_post_only(params.post_only),
        )

        ix = self.program.instruction["jit"]( # type: ignore
            jit_params,
            ctx=Context(
                accounts={
                    "state": self.drift_client.get_state_public_key(),
                    "user": self.drift_client.get_user_account_public_key(
                        sub_account_id
                    ),
                    "user_stats": self.drift_client.get_user_stats_public_key(),
                    "taker": params.taker_key,
                    "taker_stats": params.taker_stats_key,
                    "authority": self.drift_client.wallet.public_key,
                    "drift_program": self.drift_client.program_id,
                },
                signers={self.drift_client.wallet}, # type: ignore
                remaining_accounts=remaining_accounts,
            ),
        )

        tx_sig_and_slot = await self.drift_client.send_ixs(ix)

        return tx_sig_and_slot.tx_sig

    def get_price_type(self, price_type: PriceType):
        if is_variant(price_type, "Oracle"):
            return self.program.type["PriceType"].Oracle() # type: ignore
        elif is_variant(price_type, "Limit"):
            return self.program.type["PriceType"].Limit() # type: ignore
        else: 
            raise ValueError(f"Unknown price type: {str(price_type)}")
        
    def get_post_only(self, post_only: PostOnlyParams):
        if is_variant(post_only, "MustPostOnly"):
            return self.program.type["PostOnlyParam"].MustPostOnly() # type: ignore
        elif is_variant(post_only, "TryPostOnly"):
            return self.program.type["PostOnlyParam"].TryPostOnly() # type: ignore
        elif is_variant(post_only, "Slide"):
            return self.program.type["PostOnlyParam"].Slide() # type: ignore
