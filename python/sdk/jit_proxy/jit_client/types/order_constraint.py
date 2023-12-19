from __future__ import annotations
from . import market_type
import typing
from dataclasses import dataclass
from construct import Container
import borsh_construct as borsh


class OrderConstraintJSON(typing.TypedDict):
    max_position: int
    min_position: int
    market_index: int
    market_type: market_type.MarketTypeJSON


@dataclass
class OrderConstraint:
    layout: typing.ClassVar = borsh.CStruct(
        "max_position" / borsh.I64,
        "min_position" / borsh.I64,
        "market_index" / borsh.U16,
        "market_type" / market_type.layout,
    )
    max_position: int
    min_position: int
    market_index: int
    market_type: market_type.MarketTypeKind

    @classmethod
    def from_decoded(cls, obj: Container) -> "OrderConstraint":
        return cls(
            max_position=obj.max_position,
            min_position=obj.min_position,
            market_index=obj.market_index,
            market_type=market_type.from_decoded(obj.market_type),
        )

    def to_encodable(self) -> dict[str, typing.Any]:
        return {
            "max_position": self.max_position,
            "min_position": self.min_position,
            "market_index": self.market_index,
            "market_type": self.market_type.to_encodable(),
        }

    def to_json(self) -> OrderConstraintJSON:
        return {
            "max_position": self.max_position,
            "min_position": self.min_position,
            "market_index": self.market_index,
            "market_type": self.market_type.to_json(),
        }

    @classmethod
    def from_json(cls, obj: OrderConstraintJSON) -> "OrderConstraint":
        return cls(
            max_position=obj["max_position"],
            min_position=obj["min_position"],
            market_index=obj["market_index"],
            market_type=market_type.from_json(obj["market_type"]),
        )
