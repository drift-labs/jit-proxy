# Python JIT Proxy SDK

## QuickStart Guide

1. Run `poetry install` in the `/python` directory
2. Add ```.env``` file in ```/sdk/examples``` with your ```RPC_URL``` and ```PRIVATE_KEY``` as a byte array
3. Customize ```JitParams``` in either ```shotgun.py``` or ```sniper.py```
4. Run ```poetry run python -m examples.sniper``` or ```poetry run python -m examples.shotgun``` while in the ```sdk``` directory

## Shotgun vs Sniper

The ```JitterShotgun``` will immediately spray transactions when it detects an eligible auction.
It'll try up to 10 times to fulfill the order, retrying if the order doesn't yet cross or the oracle is stale.

The ```JitterSniper``` will wait until it detects an order that has a high chance of crossing the ```JitParams``` and retry up to 3 times on errors.  It won't send transactions immediately like the ```JitterShotgun``` does, but will try to determine that the order will cross the bid/ask during the auction before sending a transaction.


## How to set up a ```JitProxyClient``` and ```Jitter```

This example uses the ```JitterShotgun```, but the same logic follows for the ```JitterSniper```.

However, the ```JitterSniper``` also requires a ```SlotSubscriber``` in its constructor.

```
    jit_proxy_client = JitProxyClient(
        drift_client,
        # JIT program ID
        Pubkey.from_string("J1TnP8zvVxbtF5KFp5xRmWuvG9McnhzmBd9XGfCyuxFP"),
    )

    jitter_shotgun = JitterShotgun(drift_client, auction_subscriber, jit_proxy_client, True) # The boolean is logging verbosity, True = verbose.

    jit_params = JitParams(
        bid=-1_000_000,
        ask=1_010_000,
        min_position=0,
        max_position=2,
        price_type=PriceType.Oracle(),
        sub_account_id=None,
    )

    # Add your parameters to the Jitter before subscribing
    jitter_shotgun.update_perp_params(0, jit_params)

    await jitter_shotgun.subscribe()
```