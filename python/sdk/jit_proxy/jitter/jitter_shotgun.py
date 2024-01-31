import asyncio

from typing import Any, Coroutine

from solders.pubkey import Pubkey # type: ignore

from driftpy.drift_client import DriftClient
from driftpy.auction_subscriber.auction_subscriber import AuctionSubscriber
from driftpy.types import is_variant, UserAccount, Order, PostOnlyParams
from driftpy.accounts.get_accounts import get_user_stats_account

from jit_proxy.jitter.base_jitter import BaseJitter
from jit_proxy.jit_proxy_client import JitIxParams, JitProxyClient


class JitterShotgun(BaseJitter):
    def __init__(
        self,
        drift_client: DriftClient,
        auction_subscriber: AuctionSubscriber,
        jit_proxy_client: JitProxyClient,
        verbose: bool,
    ):
        super().__init__(drift_client, auction_subscriber, jit_proxy_client, verbose)

    async def subscribe(self):
        await super().subscribe()

    async def create_try_fill(
        self,
        taker: UserAccount,
        taker_key: Pubkey,
        taker_stats_key: Pubkey,
        order: Order,
        order_sig: str,
    ) -> Coroutine[Any, Any, None]:
        self.logger.info("JitterShotgun: Creating Try Fill")

        async def try_fill():
            taker_stats = await get_user_stats_account(
                self.drift_client.program, taker.authority
            )

            referrer_info = self.get_referrer_info(taker_stats)

            for i in range(order.auction_duration):
                params = (
                    self.perp_params.get(order.market_index)
                    if is_variant(order.market_type, "Perp")
                    else self.spot_params.get(order.market_index)
                )

                if params is None:
                    self.ongoing_auctions.pop(order_sig)
                    return

                self.logger.info(f"Trying to fill {order_sig} -> Attempt: {i + 1}")

                try:
                    if params.max_position == 0 and params.min_position == 0:
                        break
            
                    sig = await self.jit_proxy_client.jit(
                        JitIxParams(
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
                            PostOnlyParams.MustPostOnly(),
                        )
                    )

                    self.logger.info(f"Filled Order: {order_sig}")
                    self.logger.info(f"Signature: {sig}")
                    await asyncio.sleep(10)  # sleep for 10 seconds
                    del self.ongoing_auctions[order_sig]
                    return
                except Exception as e:
                    self.logger.error(f"Failed to fill Order: {order_sig}")
                    if "0x1770" in str(e) or "0x1771" in str(e):
                        self.logger.error(f"Order: {order_sig} does not cross params yet, retrying")
                    elif "0x1779" in str(e):
                        self.logger.error(f"Order: {order_sig} could not fill, retrying")
                    elif "0x1793" in str(e):
                        self.logger.error("Oracle invalid, retrying")
                    elif "0x1772" in str(e):
                        self.logger.error(f"Order: {order_sig} already filled")
                        # we don't want to retry if the order is filled
                        break
                    else:
                        self.logger.error(f"Error: {e}")
                        await asyncio.sleep(10)  # sleep for 10 seconds
                        del self.ongoing_auctions[order_sig]
                        return
            if order_sig in self.ongoing_auctions:
                del self.ongoing_auctions[order_sig]

        return await try_fill()
