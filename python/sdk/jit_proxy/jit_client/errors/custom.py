import typing
from anchorpy.error import ProgramError


class BidNotCrossed(ProgramError):
    def __init__(self) -> None:
        super().__init__(6000, "BidNotCrossed")

    code = 6000
    name = "BidNotCrossed"
    msg = "BidNotCrossed"


class AskNotCrossed(ProgramError):
    def __init__(self) -> None:
        super().__init__(6001, "AskNotCrossed")

    code = 6001
    name = "AskNotCrossed"
    msg = "AskNotCrossed"


class TakerOrderNotFound(ProgramError):
    def __init__(self) -> None:
        super().__init__(6002, "TakerOrderNotFound")

    code = 6002
    name = "TakerOrderNotFound"
    msg = "TakerOrderNotFound"


class OrderSizeBreached(ProgramError):
    def __init__(self) -> None:
        super().__init__(6003, "OrderSizeBreached")

    code = 6003
    name = "OrderSizeBreached"
    msg = "OrderSizeBreached"


class NoBestBid(ProgramError):
    def __init__(self) -> None:
        super().__init__(6004, "NoBestBid")

    code = 6004
    name = "NoBestBid"
    msg = "NoBestBid"


class NoBestAsk(ProgramError):
    def __init__(self) -> None:
        super().__init__(6005, "NoBestAsk")

    code = 6005
    name = "NoBestAsk"
    msg = "NoBestAsk"


class NoArbOpportunity(ProgramError):
    def __init__(self) -> None:
        super().__init__(6006, "NoArbOpportunity")

    code = 6006
    name = "NoArbOpportunity"
    msg = "NoArbOpportunity"


class UnprofitableArb(ProgramError):
    def __init__(self) -> None:
        super().__init__(6007, "UnprofitableArb")

    code = 6007
    name = "UnprofitableArb"
    msg = "UnprofitableArb"


class PositionLimitBreached(ProgramError):
    def __init__(self) -> None:
        super().__init__(6008, "PositionLimitBreached")

    code = 6008
    name = "PositionLimitBreached"
    msg = "PositionLimitBreached"


CustomError = typing.Union[
    BidNotCrossed,
    AskNotCrossed,
    TakerOrderNotFound,
    OrderSizeBreached,
    NoBestBid,
    NoBestAsk,
    NoArbOpportunity,
    UnprofitableArb,
    PositionLimitBreached,
]
CUSTOM_ERROR_MAP: dict[int, CustomError] = {
    6000: BidNotCrossed(),
    6001: AskNotCrossed(),
    6002: TakerOrderNotFound(),
    6003: OrderSizeBreached(),
    6004: NoBestBid(),
    6005: NoBestAsk(),
    6006: NoArbOpportunity(),
    6007: UnprofitableArb(),
    6008: PositionLimitBreached(),
}


def from_code(code: int) -> typing.Optional[CustomError]:
    maybe_err = CUSTOM_ERROR_MAP.get(code)
    if maybe_err is None:
        return None
    return maybe_err
