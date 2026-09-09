use anchor_lang::prelude::*;

#[error_code]
pub enum EscrowError {
    #[msg("Deposit amount must be greater than zero")]
    InvalidAmount,
}