use anchor_lang::prelude::*;

#[error_code]
#[derive(PartialEq, Eq)]
pub enum ErrorCode {
    #[msg("BidNotCrossed")]
    BidNotCrossed,
    #[msg("AskNotCrossed")]
    AskNotCrossed,
    #[msg("TakerOrderNotFound")]
    TakerOrderNotFound,
    #[msg("OrderSizeBreached")]
    OrderSizeBreached,
    #[msg("NoBestBid")]
    NoBestBid,
    #[msg("NoBestAsk")]
    NoBestAsk,
    #[msg("NoArbOpportunity")]
    NoArbOpportunity,
    #[msg("UnprofitableArb")]
    UnprofitableArb,
    #[msg("PositionLimitBreached")]
    PositionLimitBreached,
    #[msg("NoFill")]
    NoFill,
    #[msg("SignedMsgOrderDoesNotExist")]
    SignedMsgOrderDoesNotExist,
}
