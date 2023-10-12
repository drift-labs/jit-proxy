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
}
