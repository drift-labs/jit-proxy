use anchor_lang::prelude::*;
use borsh::{BorshDeserialize, BorshSerialize};
use drift::state::order_params::PostOnlyParam as DriftPostOnlyParam;
use drift::state::user::MarketType as DriftMarketType;

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq)]
pub enum PostOnlyParam {
    None,
    MustPostOnly, // Tx fails if order can't be post only
    TryPostOnly,  // Tx succeeds and order not placed if can't be post only
    Slide,        // Modify price to be post only if can't be post only
}

impl PostOnlyParam {
    pub fn to_drift_param(self) -> DriftPostOnlyParam {
        match self {
            PostOnlyParam::None => DriftPostOnlyParam::None,
            PostOnlyParam::MustPostOnly => DriftPostOnlyParam::MustPostOnly,
            PostOnlyParam::TryPostOnly => DriftPostOnlyParam::TryPostOnly,
            PostOnlyParam::Slide => DriftPostOnlyParam::Slide,
        }
    }
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq)]
pub enum PriceType {
    Limit,
    Oracle,
}

#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, PartialEq, Debug, Eq)]
pub enum MarketType {
    Perp,
    Spot,
}

impl MarketType {
    pub fn to_drift_param(self) -> DriftMarketType {
        match self {
            MarketType::Spot => DriftMarketType::Spot,
            MarketType::Perp => DriftMarketType::Perp,
        }
    }
}
