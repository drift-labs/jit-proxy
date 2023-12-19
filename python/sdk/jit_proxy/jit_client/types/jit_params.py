from __future__ import annotations
from . import post_only_param, price_type
import typing
from dataclasses import dataclass
from construct import Container
import borsh_construct as borsh


class JitParamsJSON(typing.TypedDict):
    taker_order_id: int
    max_position: int
    min_position: int
    bid: int
    ask: int
    price_type: price_type.PriceTypeJSON
    post_only: typing.Optional[post_only_param.PostOnlyParamJSON]


@dataclass
class JitParams:
    layout: typing.ClassVar = borsh.CStruct(
        "taker_order_id" / borsh.U32,
        "max_position" / borsh.I64,
        "min_position" / borsh.I64,
        "bid" / borsh.I64,
        "ask" / borsh.I64,
        "price_type" / price_type.layout,
        "post_only" / borsh.Option(post_only_param.layout),
    )
    taker_order_id: int
    max_position: int
    min_position: int
    bid: int
    ask: int
    price_type: price_type.PriceTypeKind
    post_only: typing.Optional[post_only_param.PostOnlyParamKind]

    @classmethod
    def from_decoded(cls, obj: Container) -> "JitParams":
        return cls(
            taker_order_id=obj.taker_order_id,
            max_position=obj.max_position,
            min_position=obj.min_position,
            bid=obj.bid,
            ask=obj.ask,
            price_type=price_type.from_decoded(obj.price_type),
            post_only=(
                None
                if obj.post_only is None
                else post_only_param.from_decoded(obj.post_only)
            ),
        )

    def to_encodable(self) -> dict[str, typing.Any]:
        return {
            "taker_order_id": self.taker_order_id,
            "max_position": self.max_position,
            "min_position": self.min_position,
            "bid": self.bid,
            "ask": self.ask,
            "price_type": self.price_type.to_encodable(),
            "post_only": (
                None if self.post_only is None else self.post_only.to_encodable()
            ),
        }

    def to_json(self) -> JitParamsJSON:
        return {
            "taker_order_id": self.taker_order_id,
            "max_position": self.max_position,
            "min_position": self.min_position,
            "bid": self.bid,
            "ask": self.ask,
            "price_type": self.price_type.to_json(),
            "post_only": (None if self.post_only is None else self.post_only.to_json()),
        }

    @classmethod
    def from_json(cls, obj: JitParamsJSON) -> "JitParams":
        return cls(
            taker_order_id=obj["taker_order_id"],
            max_position=obj["max_position"],
            min_position=obj["min_position"],
            bid=obj["bid"],
            ask=obj["ask"],
            price_type=price_type.from_json(obj["price_type"]),
            post_only=(
                None
                if obj["post_only"] is None
                else post_only_param.from_json(obj["post_only"])
            ),
        )
