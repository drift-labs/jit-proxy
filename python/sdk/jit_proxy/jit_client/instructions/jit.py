from __future__ import annotations
import typing
from solders.pubkey import Pubkey
from solders.instruction import Instruction, AccountMeta
import borsh_construct as borsh
from .. import types
from ..program_id import PROGRAM_ID


class JitArgs(typing.TypedDict):
    params: types.jit_params.JitParams


layout = borsh.CStruct("params" / types.jit_params.JitParams.layout)


class JitAccounts(typing.TypedDict):
    state: Pubkey
    user: Pubkey
    user_stats: Pubkey
    taker: Pubkey
    taker_stats: Pubkey
    authority: Pubkey
    drift_program: Pubkey


def jit(
    args: JitArgs,
    accounts: JitAccounts,
    program_id: Pubkey = PROGRAM_ID,
    remaining_accounts: typing.Optional[typing.List[AccountMeta]] = None,
) -> Instruction:
    keys: list[AccountMeta] = [
        AccountMeta(pubkey=accounts["state"], is_signer=False, is_writable=False),
        AccountMeta(pubkey=accounts["user"], is_signer=False, is_writable=True),
        AccountMeta(pubkey=accounts["user_stats"], is_signer=False, is_writable=True),
        AccountMeta(pubkey=accounts["taker"], is_signer=False, is_writable=True),
        AccountMeta(pubkey=accounts["taker_stats"], is_signer=False, is_writable=True),
        AccountMeta(pubkey=accounts["authority"], is_signer=True, is_writable=False),
        AccountMeta(
            pubkey=accounts["drift_program"], is_signer=False, is_writable=False
        ),
    ]
    if remaining_accounts is not None:
        keys += remaining_accounts
    identifier = b"c*a\x8c\x98>\xa7\xea"
    encoded_args = layout.build(
        {
            "params": args["params"].to_encodable(),
        }
    )
    data = identifier + encoded_args
    return Instruction(program_id, data, keys)
