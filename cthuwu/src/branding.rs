use crate::token_eye::U256;
use anyhow::{Result, bail};

pub const DEFAULT_INITIAL_PRICE_BASIS_POINTS: u16 = 1_000;
pub const MIN_INITIAL_PRICE_BASIS_POINTS: u16 = 500;
pub const MAX_INITIAL_PRICE_BASIS_POINTS: u16 = 2_000;
pub const WEEKLY_UPKEEP_BASIS_POINTS: u16 = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrandingQuote {
    pub treasury_balance: U256,
    pub price_basis_points: u16,
    pub initial_declared_price: U256,
    pub first_week_upkeep: U256,
}

/// Produces the exact values that must be disclosed and bound into mint consent.
///
/// The ordinary baseline is 10% of a fresh Tentacle treasury observation. A
/// caller may make a reasoned adjustment only inside the compiled 5%-20% band.
pub fn quote_initial_branding(
    treasury_balance: U256,
    price_basis_points: u16,
) -> Result<BrandingQuote> {
    if !(MIN_INITIAL_PRICE_BASIS_POINTS..=MAX_INITIAL_PRICE_BASIS_POINTS)
        .contains(&price_basis_points)
    {
        bail!("initial Branding price adjustment must remain between 5% and 20%");
    }
    let initial_declared_price = treasury_balance
        .checked_mul_basis_points(price_basis_points)
        .ok_or_else(|| anyhow::anyhow!("initial Branding price calculation overflowed"))?;
    if initial_declared_price.is_zero() {
        bail!("initial Branding price must be positive");
    }

    // The deployed contract rounds weekly upkeep upward. Compute ceil(price *
    // 10 / 10,000) without widening beyond uint256; a nonzero sub-1,000-unit
    // price therefore still owes one base unit.
    let floor_upkeep = initial_declared_price
        .checked_mul_basis_points(WEEKLY_UPKEEP_BASIS_POINTS)
        .ok_or_else(|| anyhow::anyhow!("Branding upkeep calculation overflowed"))?;
    let recomposed = floor_upkeep
        .checked_mul_u64(1_000)
        .ok_or_else(|| anyhow::anyhow!("Branding upkeep comparison overflowed"))?;
    let first_week_upkeep = if recomposed < initial_declared_price {
        add_one(floor_upkeep)
            .ok_or_else(|| anyhow::anyhow!("Branding upkeep rounded past uint256"))?
    } else {
        floor_upkeep
    };

    Ok(BrandingQuote {
        treasury_balance,
        price_basis_points,
        initial_declared_price,
        first_week_upkeep,
    })
}

fn add_one(value: U256) -> Option<U256> {
    let mut bytes = value.to_be_bytes();
    for byte in bytes.iter_mut().rev() {
        let (next, carry) = byte.overflowing_add(1);
        *byte = next;
        if !carry {
            return Some(U256::from_be_bytes(bytes));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_quote_is_ten_percent_of_current_treasury() {
        let quote = quote_initial_branding(
            U256::from_u64(1_000_000),
            DEFAULT_INITIAL_PRICE_BASIS_POINTS,
        )
        .unwrap();
        assert_eq!(quote.initial_declared_price, U256::from_u64(100_000));
        assert_eq!(quote.first_week_upkeep, U256::from_u64(100));
    }

    #[test]
    fn upkeep_matches_contract_upward_rounding() {
        let quote =
            quote_initial_branding(U256::from_u64(10), DEFAULT_INITIAL_PRICE_BASIS_POINTS).unwrap();
        assert_eq!(quote.initial_declared_price, U256::from_u64(1));
        assert_eq!(quote.first_week_upkeep, U256::from_u64(1));
    }

    #[test]
    fn adjustments_are_bounded_and_zero_quotes_are_rejected() {
        assert!(quote_initial_branding(U256::from_u64(100), 499).is_err());
        assert!(quote_initial_branding(U256::from_u64(100), 2_001).is_err());
        assert!(quote_initial_branding(U256::ZERO, DEFAULT_INITIAL_PRICE_BASIS_POINTS).is_err());
    }
}
