import asyncio

from typing import Any, Coroutine, Optional

from solders.pubkey import Pubkey

from jit_proxy.jitter.base_jitter import BaseJitter
from jit_proxy.jit_proxy_client import JitIxParams, JitProxyClient

from driftpy.drift_client import DriftClient
from driftpy.auction_subscriber.auction_subscriber import AuctionSubscriber
from driftpy.types import UserAccount, Order, UserStatsAccount, ReferrerInfo
from driftpy.accounts.get_accounts import get_user_stats_account
from driftpy.addresses import get_user_stats_account_public_key

class JitterShotgun(BaseJitter):
    def __init__(
        self, 
        drift_client: DriftClient, 
        auction_subscriber: AuctionSubscriber, 
        jit_proxy_client: JitProxyClient
        ):
        super().__init__(drift_client, auction_subscriber, jit_proxy_client)

    async def create_try_fill(
        self,
        taker: UserAccount,
        taker_key: Pubkey,
        taker_stats_key: Pubkey,
        order: Order,
        order_sig: str
    ) -> Coroutine[Any, Any, None]:
        print("Creating Try Fill")
        async def try_fill():
            i = 0
            while i < 10:
                params = self.perp_params.get(order.market_index)

                if params is None:
                    self.ongoing_auctions.pop(order_sig)
                    return
                
                taker_stats = await get_user_stats_account(self.drift_client.program, taker.authority)

                referrer_info = self.get_referrer_info(taker_stats)

                print(f"Trying to fill {order_sig}")

                try:
                    tx_sig_and_slot = await self.jit_proxy_client.jit(
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
                    print(f"tx signature: {tx_sig_and_slot.tx_sig}")
                    await asyncio.sleep(10) # sleep for 10 seconds
                    del self.ongoing_auctions[order_sig]
                    return
                except Exception as e:
                    print(f"Failed to fill {order_sig}")
                    if '0x1770' in str(e) or '0x1771' in str(e):
                        print('Order does not cross params yet, retrying')
                    elif '0x1793' in str(e):
                        print('Oracle invalid, retrying')
                    else:

                        await asyncio.sleep(10)  # Sleep for 10 seconds
                        del self.ongoing_auctions[order_sig]
                        return

        return try_fill


    def get_referrer_info(self, taker_stats: UserStatsAccount) -> Optional[ReferrerInfo]:
        if taker_stats.referrer == Pubkey.default():
            return None
        else:
            return ReferrerInfo(
                taker_stats.referrer, 
                get_user_stats_account_public_key(
                    self.drift_client.program_id, 
                    taker_stats.referrer
                    )
                )