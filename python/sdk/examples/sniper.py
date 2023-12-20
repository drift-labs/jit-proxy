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
from driftpy.slot.slot_subscriber import SlotSubscriber

from jit_proxy.jitter.jitter_sniper import JitterSniper
from jit_proxy.jitter.base_jitter import JitParams
from jit_proxy.jit_proxy_client import JitProxyClient

from jit_proxy.jit_client.types.price_type import Oracle

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
        "mainnet",             
        account_subscription = AccountSubscriptionConfig("websocket"),
    )

    auction_subscriber_config = AuctionSubscriberConfig(drift_client)
    auction_subscriber = AuctionSubscriber(auction_subscriber_config)
    
    slot_subscriber = SlotSubscriber(drift_client)

    jit_proxy_client = JitProxyClient(
        drift_client, 
        # JIT program ID
        Pubkey.from_string('J1TnP8zvVxbtF5KFp5xRmWuvG9McnhzmBd9XGfCyuxFP')
    )

    jitter_sniper = JitterSniper(
        drift_client,
        slot_subscriber,
        auction_subscriber,
        jit_proxy_client
    )

    jit_params = JitParams(
        bid = 1_000_000,
        ask = 1_010_000,
        min_position = 1,
        max_position = 2,
        price_type = Oracle(),
        sub_account_id = None
    )

    jitter_sniper.update_spot_params(1, jit_params)
    jitter_sniper.update_perp_params(1, jit_params)

    print(f"Added JitParams: {jit_params} to JitterSniper")

    await jitter_sniper.subscribe()

    print("Subscribed to JitterSniper successfully!")

    # quick & dirty way to keep event loop open 
    try:
        while True:
            await asyncio.sleep(3600)
    except asyncio.CancelledError:
        pass

if __name__ == "__main__":
    asyncio.run(main())