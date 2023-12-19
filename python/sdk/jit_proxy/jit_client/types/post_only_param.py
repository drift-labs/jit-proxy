from __future__ import annotations
import typing
from dataclasses import dataclass
from anchorpy.borsh_extension import EnumForCodegen
import borsh_construct as borsh


class NoneJSON(typing.TypedDict):
    kind: typing.Literal["None"]


class MustPostOnlyJSON(typing.TypedDict):
    kind: typing.Literal["MustPostOnly"]


class TryPostOnlyJSON(typing.TypedDict):
    kind: typing.Literal["TryPostOnly"]


@dataclass
class None_:
    discriminator: typing.ClassVar = 0
    kind: typing.ClassVar = "None"

    @classmethod
    def to_json(cls) -> NoneJSON:
        return NoneJSON(
            kind="None",
        )

    @classmethod
    def to_encodable(cls) -> dict:
        return {
            "None": {},
        }


@dataclass
class MustPostOnly:
    discriminator: typing.ClassVar = 1
    kind: typing.ClassVar = "MustPostOnly"

    @classmethod
    def to_json(cls) -> MustPostOnlyJSON:
        return MustPostOnlyJSON(
            kind="MustPostOnly",
        )

    @classmethod
    def to_encodable(cls) -> dict:
        return {
            "MustPostOnly": {},
        }


@dataclass
class TryPostOnly:
    discriminator: typing.ClassVar = 2
    kind: typing.ClassVar = "TryPostOnly"

    @classmethod
    def to_json(cls) -> TryPostOnlyJSON:
        return TryPostOnlyJSON(
            kind="TryPostOnly",
        )

    @classmethod
    def to_encodable(cls) -> dict:
        return {
            "TryPostOnly": {},
        }


PostOnlyParamKind = typing.Union[None_, MustPostOnly, TryPostOnly]
PostOnlyParamJSON = typing.Union[NoneJSON, MustPostOnlyJSON, TryPostOnlyJSON]


def from_decoded(obj: dict) -> PostOnlyParamKind:
    if not isinstance(obj, dict):
        raise ValueError("Invalid enum object")
    if "None" in obj:
        return None_()
    if "MustPostOnly" in obj:
        return MustPostOnly()
    if "TryPostOnly" in obj:
        return TryPostOnly()
    raise ValueError("Invalid enum object")


def from_json(obj: PostOnlyParamJSON) -> PostOnlyParamKind:
    if obj["kind"] == "None":
        return None_()
    if obj["kind"] == "MustPostOnly":
        return MustPostOnly()
    if obj["kind"] == "TryPostOnly":
        return TryPostOnly()
    kind = obj["kind"]
    raise ValueError(f"Unrecognized enum kind: {kind}")


layout = EnumForCodegen(
    "None" / borsh.CStruct(),
    "MustPostOnly" / borsh.CStruct(),
    "TryPostOnly" / borsh.CStruct(),
)
