use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Deposit amount must be greater than zero")]
    InvalidAmount,
    #[msg("Withdrawal would leave vault below rent-exempt minimum")]
    InsufficientVaultBalance,
}