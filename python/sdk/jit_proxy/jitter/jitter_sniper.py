from dataclasses import dataclass

from driftpy.types import OraclePriceData

from jit_proxy.jitter.base_jitter import BaseJitter

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

# class JitterSniper(BaseJitter):