import asyncio
import os

from dotenv import load_dotenv

from solana.rpc.async_api import AsyncClient
from solders.pubkey import Pubkey

from anchorpy import Wallet

from driftpy.drift_client import DriftClient
from driftpy.account_subscription_config import AccountSubscriptionConfig
from driftpy.auction_subscriber.auction_subscriber import AuctionSubscriber
from driftpy.auction_subscriber.types import AuctionSubscriberConfig
from driftpy.keypair import load_keypair

from jit_proxy.jitter.jitter_shotgun import JitterShotgun
from jit_proxy.jitter.base_jitter import JitParams
from jit_proxy.jit_proxy_client import JitProxyClient, PriceType


async def main():
    load_dotenv()
    secret = os.getenv("PRIVATE_KEY")
    url = os.getenv("RPC_URL")

    kp = load_keypair(secret)
    wallet = Wallet(kp)

    connection = AsyncClient(url)
    drift_client = DriftClient(
        connection,
        wallet,
        "mainnet",
        account_subscription=AccountSubscriptionConfig("websocket"),
    )

    auction_subscriber_config = AuctionSubscriberConfig(drift_client)
    auction_subscriber = AuctionSubscriber(auction_subscriber_config)

    jit_proxy_client = JitProxyClient(
        drift_client,
        # JIT program ID
        Pubkey.from_string("J1TnP8zvVxbtF5KFp5xRmWuvG9McnhzmBd9XGfCyuxFP"),
    )

    jitter_shotgun = JitterShotgun(drift_client, auction_subscriber, jit_proxy_client)

    jit_params = JitParams(
        bid=-1_000_000,
        ask=1_010_000,
        min_position=0,
        max_position=2,
        price_type=PriceType.Oracle(),
        sub_account_id=None,
    )

    jitter_shotgun.update_spot_params(0, jit_params)
    jitter_shotgun.update_perp_params(0, jit_params)

    print(f"Added JitParams: {jit_params} to JitterShotgun")

    await jitter_shotgun.subscribe()

    print("Subscribed to JitterShotgun successfully!")

    # quick & dirty way to keep event loop open
    try:
        while True:
            await asyncio.sleep(3600)
    except asyncio.CancelledError:
        pass


if __name__ == "__main__":
    asyncio.run(main())
