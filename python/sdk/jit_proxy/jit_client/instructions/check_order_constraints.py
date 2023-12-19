from __future__ import annotations
import typing
from solders.pubkey import Pubkey
from solders.instruction import Instruction, AccountMeta
from construct import Construct
import borsh_construct as borsh
from .. import types
from ..program_id import PROGRAM_ID


class CheckOrderConstraintsArgs(typing.TypedDict):
    constraints: list[types.order_constraint.OrderConstraint]


layout = borsh.CStruct(
    "constraints"
    / borsh.Vec(typing.cast(Construct, types.order_constraint.OrderConstraint.layout))
)


class CheckOrderConstraintsAccounts(typing.TypedDict):
    user: Pubkey


def check_order_constraints(
    args: CheckOrderConstraintsArgs,
    accounts: CheckOrderConstraintsAccounts,
    program_id: Pubkey = PROGRAM_ID,
    remaining_accounts: typing.Optional[typing.List[AccountMeta]] = None,
) -> Instruction:
    keys: list[AccountMeta] = [
        AccountMeta(pubkey=accounts["user"], is_signer=False, is_writable=False)
    ]
    if remaining_accounts is not None:
        keys += remaining_accounts
    identifier = b"\xb7\xae\x8e\xf5\x05\x1d\xcf\x02"
    encoded_args = layout.build(
        {
            "constraints": list(
                map(lambda item: item.to_encodable(), args["constraints"])
            ),
        }
    )
    data = identifier + encoded_args
    return Instruction(program_id, data, keys)
