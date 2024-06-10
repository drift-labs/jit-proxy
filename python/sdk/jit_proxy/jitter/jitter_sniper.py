import asyncio

from dataclasses import dataclass
from typing import Any, Coroutine

from solders.pubkey import Pubkey # type: ignore

from driftpy.drift_client import DriftClient
from driftpy.auction_subscriber.auction_subscriber import AuctionSubscriber
from driftpy.slot.slot_subscriber import SlotSubscriber
from driftpy.accounts.get_accounts import get_user_stats_account
from driftpy.types import is_variant, OraclePriceData, Order, UserAccount, PostOnlyParams
from driftpy.math.conversion import convert_to_number
from driftpy.math.auction import (
    get_auction_price_for_oracle_offset_auction,
    get_auction_price,
)
from driftpy.constants.numeric_constants import PRICE_PRECISION

from jit_proxy.jitter.base_jitter import BaseJitter
from jit_proxy.jit_proxy_client import JitProxyClient


@dataclass
class AuctionAndOrderDetails:
    slots_until_cross: int
    will_cross: bool
    bid: int
    ask: int
    auction_start_price: int
    auction_end_price: int
    step_size: int
    oracle_price: OraclePriceData


class JitterSniper(BaseJitter):
    def __init__(
        self,
        drift_client: DriftClient,
        slot_subscriber: SlotSubscriber,
        auction_subscriber: AuctionSubscriber,
        jit_proxy_client: JitProxyClient,
        verbose: bool,
    ):
        super().__init__(drift_client, auction_subscriber, jit_proxy_client, verbose)
        self.slot_subscriber = slot_subscriber

    async def subscribe(self):
        await super().subscribe()
        await self.slot_subscriber.subscribe()

    async def create_try_fill(
        self,
        taker: UserAccount,
        taker_key: Pubkey,
        taker_stats_key: Pubkey,
        order: Order,
        order_sig: str,
    ) -> Coroutine[Any, Any, None]:
        self.logger.info("JitterSniper: Creating Try Fill")

        async def try_fill():
            params = (
                self.perp_params.get(order.market_index)
                if is_variant(order.market_type, "Perp")
                else self.spot_params.get(order.market_index)
            )

            if params is None:
                del self.ongoing_auctions[order_sig]
                return

            taker_stats = await get_user_stats_account(
                self.drift_client.program, taker.authority
            )

            referrer_info = self.get_referrer_info(taker_stats)

            details = self.get_auction_and_order_details(order)

            if is_variant(order.market_type, "Perp"):
                current_perp_pos = self.drift_client.get_user().get_perp_position(
                    order.market_index
                )
                if current_perp_pos.base_asset_amount < 0 and is_variant(
                    order.direction, "Short"
                ):
                    if current_perp_pos.base_asset_amount <= params.min_position:
                        self.logger.warning(
                            f"Order would increase existing short (mkt \
                              {str(order.market_type)}-{order.market_index} \
                              ) too much"
                        )
                        del self.ongoing_auctions[order_sig]
                        return
                    elif current_perp_pos.base_asset_amount > 0 and is_variant(
                        order.direction, "Long"
                    ):
                        if current_perp_pos.base_asset_amount >= params.max_position:
                            self.logger.warning(
                                f"Order would increase existing long (mkt \
                              {str(order.market_type)}-{order.market_index} \
                              ) too much"
                            )
                        del self.ongoing_auctions[order_sig]
                        return

            self.logger.info(
                f"""
                Taker wants to {order.direction}, order slot is {order.slot},
                My market: {details.bid}@{details.ask},
                Auction: {details.auction_start_price} -> {details.auction_end_price}, step size {details.step_size}
                Current slot: {self.slot_subscriber.current_slot}, Order slot: {order.slot},
                Will cross?: {details.will_cross}
                Slots to wait: {details.slots_until_cross}. Target slot = {order.slot + details.slots_until_cross}
            """
            )

            target_slot = (
                order.slot + details.slots_until_cross
                if details.will_cross
                else order.slot + order.auction_duration + 1
            )

            (slot, updated_details) = await self.wait_for_slot_or_cross_or_expiry(
                target_slot, order, details
            )

            if slot == -1:
                self.logger.info("Auction expired without crossing")
                if order_sig in self.ongoing_auctions:
                    del self.ongoing_auctions[order_sig]
                return

            params = (
                self.perp_params.get(order.market_index)
                if is_variant(order.market_type, "Perp")
                else self.spot_params.get(order.market_index)
            )

            bid = (
                convert_to_number(details.oracle_price + params.bid, PRICE_PRECISION)
                if is_variant(params.price_type, "Oracle")
                else convert_to_number(params.bid, PRICE_PRECISION)
            )

            ask = (
                convert_to_number(details.oracle_price + params.ask, PRICE_PRECISION)
                if is_variant(params.price_type, "Oracle")
                else convert_to_number(params.ask, PRICE_PRECISION)
            )

            auction_price = convert_to_number(
                get_auction_price(order, slot, updated_details.oracle_price.price),
                PRICE_PRECISION,
            )

            self.logger.info(
                f"""
                Expected auction price: {details.auction_start_price + details.slots_until_cross * details.step_size}
                Actual auction price: {auction_price}
                -----------------
                Looking for slot {order.slot + details.slots_until_cross}
                Got slot {slot}
            """
            )

            self.logger.info(
                f"""
                Trying to fill {order_sig} with:
                market: {bid}@{ask}
                auction price: {auction_price}
                submitting: {convert_to_number(params.bid, PRICE_PRECISION)}@{convert_to_number(params.ask, PRICE_PRECISION)}
            """
            )

            for _ in range(3):
                try:
                    if params.max_position == 0 and params.min_position == 0:
                        break
            
                    tx_sig_and_slot = await self.jit_proxy_client.jit(
                        {
                            taker_key,
                            taker_stats_key,
                            taker,
                            order.order_id,
                            params.max_position,
                            params.min_position,
                            params.bid,
                            params.ask,
                            params.price_type,
                            referrer_info,
                            params.sub_account_id,
                            PostOnlyParams.TryPostOnly(),
                        }
                    )

                    self.logger.info(f"Filled {order_sig}")
                    self.logger.info(f"tx signature: {tx_sig_and_slot.tx_sig}")
                    await asyncio.sleep(3)  # Sleep for 3 seconds
                    del self.ongoing_auctions[order_sig]
                    return
                except Exception as e:
                    self.logger.error(f"Failed to fill {order_sig}: {e}")
                    if "0x1770" in str(e) or "0x1771" in str(e):
                        self.logger.error("Order does not cross params yet")
                    elif "0x1779" in str(e):
                        self.logger.error("Order could not fill, retrying")
                    elif "0x1793" in str(e):
                        self.logger.error("Oracle invalid")
                    elif "0x1772" in str(e):
                        self.logger.error("Order already filled")
                        # we don't want to retry if the order is filled
                        break
                    else:
                        await asyncio.sleep(3)  # sleep for 3 seconds
                        del self.ongoing_auctions[order_sig]
                        return

                await asyncio.sleep(0.05)  # 50ms
            if order_sig in self.ongoing_auctions:
                del self.on_going_auctions[order_sig]

        return await try_fill()

    def get_auction_and_order_details(self, order: Order) -> AuctionAndOrderDetails:
        params = (
            self.perp_params.get(order.market_index)
            if is_variant(order.market_type, "Perp")
            else self.spot_params.get(order.market_index)
        )

        oracle_price = (
            self.drift_client.get_oracle_price_data_for_perp_market(order.market_index)
            if is_variant(order.market_type, "Perp")
            else self.drift_client.get_oracle_price_data_for_spot_market(
                order.market_index
            )
        )

        maker_order_dir = "sell" if is_variant(order.direction, "Long") else "buy"

        auction_start_price = convert_to_number(
            get_auction_price_for_oracle_offset_auction(
                order, order.slot, oracle_price.price # type: ignore
            )
            if is_variant(order.order_type, "Oracle")
            else order.auction_start_price,
            PRICE_PRECISION,
        )

        auction_end_price = convert_to_number(
            get_auction_price_for_oracle_offset_auction(
                order, order.slot + order.auction_duration - 1, oracle_price.price # type: ignore
            )
            if is_variant(order.order_type, "Oracle")
            else order.auction_end_price,
            PRICE_PRECISION,
        )

        bid = (
            convert_to_number(oracle_price.price + params.bid, PRICE_PRECISION) # type: ignore
            if is_variant(params.price_type, "Oracle") # type: ignore
            else convert_to_number(params.bid, PRICE_PRECISION) # type: ignore
        )

        ask = (
            convert_to_number(oracle_price.price + params.ask, PRICE_PRECISION) # type: ignore
            if is_variant(params.price_type, "Oracle") # type: ignore
            else convert_to_number(params.ask, PRICE_PRECISION) # type: ignore
        )

        slots_until_cross = 0
        will_cross = False
        step_size = (auction_end_price - auction_start_price) // (
            order.auction_duration - 1
        )

        while slots_until_cross < order.auction_duration:
            if maker_order_dir == "buy":
                if (
                    convert_to_number(
                        get_auction_price(
                            order, order.slot + slots_until_cross, oracle_price.price # type: ignore
                        ),
                        PRICE_PRECISION,
                    )
                    <= bid
                ):
                    will_cross = True
                    break
            else:
                if (
                    convert_to_number(
                        get_auction_price(
                            order, order.slot + slots_until_cross, oracle_price.price # type: ignore
                        ),
                        PRICE_PRECISION,
                    )
                    >= ask
                ):
                    will_cross = True
                    break
            slots_until_cross += 1

        return AuctionAndOrderDetails(
            slots_until_cross,
            will_cross,
            bid,
            ask,
            auction_start_price,
            auction_end_price,
            step_size,
            oracle_price, # type: ignore
        )

    async def wait_for_slot_or_cross_or_expiry(
        self, target_slot: int, order: Order, initial_details: AuctionAndOrderDetails
    ) -> (int, AuctionAndOrderDetails): # type: ignore
        auction_end_slot = order.auction_duration + order.slot
        current_details: AuctionAndOrderDetails = initial_details
        will_cross = initial_details.will_cross

        if self.slot_subscriber.current_slot > auction_end_slot:
            return (-1, current_details)

        def slot_listener(slot):
            if slot >= target_slot and will_cross:
                return (slot, current_details)

        self.slot_subscriber.event_emitter.on_slot_change += slot_listener

        while True:
            if self.slot_subscriber.current_slot >= auction_end_slot:
                self.slot_subscriber.event_emitter.on_slot_change -= slot_listener
                return (-1, current_details)

            current_details = self.get_auction_and_order_details(order)
            will_cross = current_details.will_cross
            if will_cross:
                target_slot = order.slot + current_details.slots_until_cross

            await asyncio.sleep(0.05)  # 50 ms
