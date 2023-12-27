import asyncio
import logging

from typing import Callable, Dict, Optional
from abc import ABC, abstractmethod
from dataclasses import dataclass

from solders.pubkey import Pubkey

from driftpy.types import is_variant, UserAccount, Order, UserStatsAccount, ReferrerInfo
from driftpy.drift_client import DriftClient
from driftpy.auction_subscriber.auction_subscriber import AuctionSubscriber
from driftpy.addresses import get_user_stats_account_public_key
from driftpy.math.orders import has_auction_price
from driftpy.math.conversion import convert_to_number

from jit_proxy.jit_proxy_client import JitProxyClient, PriceType

UserFilter = Callable[[UserAccount, str, Order], bool]

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

@dataclass
class JitParams:
    bid: int
    ask: int
    min_position: int
    max_position: int
    price_type: PriceType
    sub_account_id: Optional[int]


class BaseJitter(ABC):
    @abstractmethod
    def __init__(
        self,
        drift_client: DriftClient,
        auction_subscriber: AuctionSubscriber,
        jit_proxy_client: JitProxyClient,
    ):
        self.drift_client = drift_client
        self.auction_subscriber = auction_subscriber
        self.jit_proxy_client = jit_proxy_client
        self.perp_params: Dict[int, JitParams] = {}
        self.spot_params: Dict[int, JitParams] = {}
        self.ongoing_auctions: Dict[str, asyncio.Future] = {}
        self.user_filter: Optional[UserFilter] = None

    @abstractmethod
    async def subscribe(self):
        await self.drift_client.subscribe()
        await self.auction_subscriber.subscribe()

        self.auction_subscriber.event_emitter.on_account_update += (
            self.on_account_update_sync
        )

    def on_account_update_sync(self, taker: UserAccount, taker_key: Pubkey, slot: int):
        asyncio.create_task(self.on_account_update(taker, taker_key, slot))

    async def on_account_update(self, taker: UserAccount, taker_key: Pubkey, slot: int):
        logger.info("Auction received!")
        logger.info("----------------------------")
        taker_key_str = str(taker_key)

        taker_stats_key = get_user_stats_account_public_key(
            self.drift_client.program_id, taker.authority
        )

        logger.info(f"Taker: {taker.authority}")

        for order in taker.orders:
            if not is_variant(order.status, "Open"):
                logger.info("Order is closed.")
                logger.info("----------------------------")
                continue

            if not has_auction_price(order, slot):
                logger.info("Order does not have auction price.")
                logger.info("----------------------------")
                continue

            if self.user_filter is not None:
                if self.user_filter(taker, taker_key_str, order):
                    logger.info("User filtered out.")
                    return

            order_sig = self.get_order_signatures(taker_key_str, order.order_id)

            logger.info(f"Order sig: {order_sig}")

            if order_sig in self.ongoing_auctions:
                continue

            logger.info(f"Market Type: {str(order.market_type)}")
            logger.info(f"Market Index: {order.market_index}")
            logger.info(f"Order Price: {convert_to_number(order.price)}")
            logger.info(f"Order Type: {str(order.order_type)}")
            logger.info(f"Order Direction: {str(order.direction)}")
            logger.info(
                f"Auction Start Price: {convert_to_number(order.auction_start_price)}"
            )
            logger.info(f"Auction End Price: {convert_to_number(order.auction_end_price)}")
            logger.info(
                f"Order Base Asset Amount: {convert_to_number(order.base_asset_amount)}"
            )
            logger.info(
                f"Order Base Asset Amount Filled: {convert_to_number(order.base_asset_amount_filled)}"
            )

            if is_variant(order.order_type, "Perp"):
                logger.info("Perp Auction")
                if not order.market_index in self.perp_params:
                    logger.info(f"Jitter not listening to {order.market_index}")
                    return

                perp_market_account = self.drift_client.get_perp_market_account(
                    order.market_index
                )

                if (
                    order.base_asset_amount - order.base_asset_amount_filled
                    <= perp_market_account.amm.min_order_size
                ):
                    logger.info("Order filled within min_order_size")
                    logger.info("----------------------------")
                    return

                future = asyncio.create_task(
                    self.create_try_fill(
                        taker, taker_key, taker_stats_key, order, order_sig
                    )
                )
                self.ongoing_auctions[order_sig] = future

            else:
                logger.info("Spot Auction")
                if not order.market_index in self.spot_params:
                    logger.info(f"Jitter not listening to {order.market_index}")
                    logger.info("----------------------------")
                    return

                spot_market_account = self.drift_client.get_spot_market_account(
                    order.market_index
                )

                if (
                    order.base_asset_amount - order.base_asset_amount_filled
                    <= spot_market_account.min_order_size
                ):
                    logger.info("Order filled within min_order_size")
                    logger.info("----------------------------")
                    return

                future = asyncio.create_task(
                    self.create_try_fill(
                        taker, taker_key, taker_stats_key, order, order_sig
                    )
                )
                self.ongoing_auctions[order_sig] = future

    @abstractmethod
    async def create_try_fill(
        self,
        taker: UserAccount,
        taker_key: Pubkey,
        taker_stats_key: Pubkey,
        order: Order,
        order_sig: str,
    ):
        future = asyncio.Future()
        future.set_result(None)
        return future

    def get_order_signatures(self, taker_key: str, order_id: int) -> str:
        return f"{taker_key}-{order_id}"

    def update_perp_params(self, market_index: int, params: JitParams):
        self.perp_params[market_index] = params

    def update_spot_params(self, market_index: int, params: JitParams):
        self.spot_params[market_index] = params

    def set_user_filter(self, user_filter: Optional[UserFilter]):
        self.user_filter = user_filter

    def get_referrer_info(
        self, taker_stats: UserStatsAccount
    ) -> Optional[ReferrerInfo]:
        if taker_stats.referrer == Pubkey.default():
            return None
        else:
            return ReferrerInfo(
                taker_stats.referrer,
                get_user_stats_account_public_key(
                    self.drift_client.program_id, taker_stats.referrer
                ),
            )
