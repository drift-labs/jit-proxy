import asyncio
import os

from dotenv import load_dotenv

from solana.rpc.async_api import AsyncClient
from solders.keypair import Keypair
from solders.pubkey import Pubkey

from anchorpy import Wallet

from driftpy.drift_client import DriftClient
from driftpy.account_subscription_config import AccountSubscriptionConfig
from driftpy.auction_subscriber.auction_subscriber import AuctionSubscriber
from driftpy.auction_subscriber.types import AuctionSubscriberConfig

from jit_proxy.jitter.jitter_shotgun import JitterShotgun
from jit_proxy.jit_proxy_client import JitProxyClient

async def main():
    load_dotenv()
    secret = os.getenv('PRIVATE_KEY')
    url = os.getenv('RPC_URL')

    pk_stripped = secret.strip('[]').replace(' ', '').split(',')
    pk_bytes = bytes([int(b) for b in pk_stripped])
    kp = Keypair.from_bytes(pk_bytes)
    wallet = Wallet(kp)


    connection = AsyncClient(url)
    drift_client = DriftClient(
        connection,
        wallet, 
        "devnet",             
        account_subscription = AccountSubscriptionConfig("websocket"),
    )
    # await drift_client.subscribe()

    auction_subscriber_config = AuctionSubscriberConfig(drift_client)
    auction_subscriber = AuctionSubscriber(auction_subscriber_config)

    jit_proxy_client = JitProxyClient(
        drift_client, 
        Pubkey.from_string('J1TnP8zvVxbtF5KFp5xRmWuvG9McnhzmBd9XGfCyuxFP')
        )

    jitter = JitterShotgun(
        drift_client,
        auction_subscriber,
        jit_proxy_client
    )

    await jitter.subscribe()

if __name__ == "__test__":
    asyncio.run(main())