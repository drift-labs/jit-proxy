from asyncio import Future
from typing import Callable, Dict, Optional
from abc import ABC, abstractmethod
from functools import partial

from solders.pubkey import Pubkey

from src.jit_proxy_client import JitParams, JitProxyClient

from driftpy.types import is_variant, UserAccount, Order
from driftpy.drift_client import DriftClient
from driftpy.auction_subscriber.auction_subscriber import AuctionSubscriber
from driftpy.addresses import get_user_stats_account_public_key
from driftpy.math.orders import has_auction_price

UserFilter = Callable[[UserAccount, str, Order], bool]

class BaseJitter(ABC):
    @abstractmethod
    def __init__(
        self,
        drift_client: DriftClient, 
        auction_subscriber: AuctionSubscriber,
        jit_proxy_client: JitProxyClient
        ):
        self.drift_client = drift_client
        self.auction_subscriber = auction_subscriber
        self.jit_proxy_client = jit_proxy_client
        self.perp_params: Dict[int, JitParams] = {}
        self.spot_params: Dict[int, JitParams] = {}
        self.ongoing_auctions: Dict[str, Future] = {}
        self.user_filter = UserFilter

    async def subscribe(self):
        await self.drift_client.subscribe()
        await self.auction_subscriber.subscribe()
        
        self.auction_subscriber.event_emitter.on("on_account_update", self.on_account_update)

    
    async def on_account_update(self, taker: UserAccount, taker_key: Pubkey, slot: int):
        taker_key_str = str(taker_key)

        taker_stats_key = get_user_stats_account_public_key(
            self.drift_client.program_id,
            taker.authority
        )

        for order in taker.orders:
            if not is_variant(order.status, 'Open'):
                continue

            if not has_auction_price(order, slot):
                continue

            if self.user_filter is not None:
                if self.user_filter(taker, taker_key_str, order):
                    return
            
            order_sig = self.get_order_signatures(taker_key_str, order.order_id)

            if order_sig in self.ongoing_auctions:
                continue

            if is_variant(order.order_type, 'Perp'):
                if not order.market_index in self.perp_params:
                    return
            
                perp_market_account = self.drift_client.get_perp_market_account(order.market_index)

                if order.base_asset_amount - order.base_asset_amount_filled <= perp_market_account.amm.min_order_size:
                    return
                
                future = await self.create_try_fill(
                    taker,
                    taker_key,
                    taker_stats_key,
                    order,
                    order_sig
                )
                self.ongoing_auctions[order_sig] = future

            else:
                if not order.market_index in self.spot_params:
                    return
                
                spot_market_account = self.drift_client.get_spot_market_account(order.market_index)

                if order.base_asset_amount - order.base_asset_amount_filled <= spot_market_account.min_order_size:
                    return
                
                future = await self.create_try_fill(
                    taker,
                    taker_key,
                    taker_stats_key,
                    order,
                    order_sig
                )
                self.ongoing_auctions[order_sig] = future

    @abstractmethod
    async def create_try_fill(
        self,
        taker: UserAccount,
        taker_key: Pubkey,
        taker_stats_key: Pubkey,
        order: Order,
        order_sig: str
    ):
        future = Future()
        future.set_result(None)
        return future

    def get_order_signatures(taker_key: str, order_id: int) -> str:
        return f"{taker_key}-{order_id}"

    def update_perp_params(self, market_index: int, params: JitParams):
        self.perp_params[market_index] = params
    
    def update_spot_params(self, market_index: int, params: JitParams):
        self.spot_params[market_index] = params

    def set_user_filter(self, user_filter: Optional[UserFilter]):
        self.user_filter = user_filter