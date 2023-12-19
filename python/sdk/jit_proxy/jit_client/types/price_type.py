from __future__ import annotations
import typing
from dataclasses import dataclass
from anchorpy.borsh_extension import EnumForCodegen
import borsh_construct as borsh


class LimitJSON(typing.TypedDict):
    kind: typing.Literal["Limit"]


class OracleJSON(typing.TypedDict):
    kind: typing.Literal["Oracle"]


@dataclass
class Limit:
    discriminator: typing.ClassVar = 0
    kind: typing.ClassVar = "Limit"

    @classmethod
    def to_json(cls) -> LimitJSON:
        return LimitJSON(
            kind="Limit",
        )

    @classmethod
    def to_encodable(cls) -> dict:
        return {
            "Limit": {},
        }


@dataclass
class Oracle:
    discriminator: typing.ClassVar = 1
    kind: typing.ClassVar = "Oracle"

    @classmethod
    def to_json(cls) -> OracleJSON:
        return OracleJSON(
            kind="Oracle",
        )

    @classmethod
    def to_encodable(cls) -> dict:
        return {
            "Oracle": {},
        }


PriceTypeKind = typing.Union[Limit, Oracle]
PriceTypeJSON = typing.Union[LimitJSON, OracleJSON]


def from_decoded(obj: dict) -> PriceTypeKind:
    if not isinstance(obj, dict):
        raise ValueError("Invalid enum object")
    if "Limit" in obj:
        return Limit()
    if "Oracle" in obj:
        return Oracle()
    raise ValueError("Invalid enum object")


def from_json(obj: PriceTypeJSON) -> PriceTypeKind:
    if obj["kind"] == "Limit":
        return Limit()
    if obj["kind"] == "Oracle":
        return Oracle()
    kind = obj["kind"]
    raise ValueError(f"Unrecognized enum kind: {kind}")


layout = EnumForCodegen("Limit" / borsh.CStruct(), "Oracle" / borsh.CStruct())
