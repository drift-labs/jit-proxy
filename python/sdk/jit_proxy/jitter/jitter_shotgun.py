import asyncio

from typing import Any, Coroutine

from solders.pubkey import Pubkey

from driftpy.drift_client import DriftClient
from driftpy.auction_subscriber.auction_subscriber import AuctionSubscriber
from driftpy.types import is_variant, UserAccount, Order
from driftpy.accounts.get_accounts import get_user_stats_account

from jit_proxy.jitter.base_jitter import BaseJitter
from jit_proxy.jit_proxy_client import JitIxParams, JitProxyClient

class JitterShotgun(BaseJitter):
    def __init__(
        self, 
        drift_client: DriftClient, 
        auction_subscriber: AuctionSubscriber, 
        jit_proxy_client: JitProxyClient
        ):
        super().__init__(drift_client, auction_subscriber, jit_proxy_client)

    async def subscribe(self):
        await super().subscribe()

    async def create_try_fill(
        self,
        taker: UserAccount,
        taker_key: Pubkey,
        taker_stats_key: Pubkey,
        order: Order,
        order_sig: str
    ) -> Coroutine[Any, Any, None]:
        print("JitterShotgun: Creating Try Fill")
        async def try_fill():
            for _ in range(10):
                params = self.perp_params.get(order.market_index) \
                    if is_variant(order.market_type, 'Perp') \
                    else self.spot_params.get(order.market_index)
                
                if params is None:
                    self.ongoing_auctions.pop(order_sig)
                    return
                
                taker_stats = await get_user_stats_account(self.drift_client.program, taker.authority)

                referrer_info = self.get_referrer_info(taker_stats)

                print(f"Trying to fill {order_sig}")

                try:
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
                            None,
                            params.price_type,
                            referrer_info,
                            params.sub_account_id
                        )
                    )

                    print(f"Filled {order_sig}")
                    print(f"Signature: {sig}")
                    await asyncio.sleep(10) # sleep for 10 seconds
                    del self.ongoing_auctions[order_sig]
                    return
                except Exception as e:
                    print(f"Failed to fill {order_sig}")
                    if '0x1770' in str(e) or '0x1771' in str(e):
                        print('Order does not cross params yet, retrying')
                    elif '0x1793' in str(e):
                        print('Oracle invalid, retrying')
                    elif '0x1772' in str(e):
                        print('Order already filled, retrying')
                    else:
                        await asyncio.sleep(10)  # sleep for 10 seconds
                        del self.ongoing_auctions[order_sig]
                        return
            del self.ongoing_auctions[order_sig]
        return await try_fill()