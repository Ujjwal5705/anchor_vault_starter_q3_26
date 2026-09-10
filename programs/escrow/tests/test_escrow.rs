use {
    anchor_lang::{
        prelude::Pubkey,
        solana_program::instruction::Instruction,
        InstructionData, ToAccountMetas,
    },
    anchor_spl::{
        associated_token::{
            get_associated_token_address_with_program_id,
            spl_associated_token_account::instruction::create_associated_token_account,
        },
        token_interface::spl_token_2022::{
            self,
            instruction::{initialize_mint2, mint_to},
            state::Mint as SplMint,
        },
    },
    escrow::ESCROW_SEED,
    litesvm::LiteSVM,
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_program_pack::Pack,
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
};

const SEED: u64 = 1;
const DEPOSIT_AMOUNT: u64 = 1_000_000;
const RECEIVE_AMOUNT: u64 = 2_000_000;
const DECIMALS: u8 = 6;

fn send(svm: &mut LiteSVM, payer: &Keypair, ix: Instruction) {
    let blockhash = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[payer]).unwrap();
    svm.send_transaction(tx).unwrap_or_else(|err| {
        panic!("tx failed: {err:?}\nlogs: {logs:#?}", logs = err.meta.logs);
    });
}

fn send_multi(svm: &mut LiteSVM, payer: &Keypair, signers: &[&Keypair], ixs: &[Instruction]) {
    let blockhash = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(ixs, Some(&payer.pubkey()), &blockhash);
    let mut all_signers = vec![payer];
    all_signers.extend_from_slice(signers);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &all_signers).unwrap();
    svm.send_transaction(tx).unwrap_or_else(|err| {
        panic!("tx failed: {err:?}\nlogs: {logs:#?}", logs = err.meta.logs);
    });
}

/// Creates a new Token-2022 mint and mints `amount` of it into `owner`'s ATA
/// (creating that ATA too). Returns the mint pubkey.
fn create_mint_and_fund(
    svm: &mut LiteSVM,
    payer: &Keypair,
    mint_authority: &Keypair,
    owner: &Pubkey,
    amount: u64,
) -> Pubkey {
    let mint = Keypair::new();
    let token_program = spl_token_2022::id();
    let rent = svm.minimum_balance_for_rent_exemption(SplMint::LEN);

    let create_account_ix = anchor_lang::solana_program::system_instruction::create_account(
        &payer.pubkey(),
        &mint.pubkey(),
        rent,
        SplMint::LEN as u64,
        &token_program,
    );

    let init_mint_ix = initialize_mint2(
        &token_program,
        &mint.pubkey(),
        &mint_authority.pubkey(),
        None,
        DECIMALS,
    )
    .unwrap();

    send_multi(svm, payer, &[&mint], &[create_account_ix, init_mint_ix]);

    let ata = get_associated_token_address_with_program_id(owner, &mint.pubkey(), &token_program);
    let create_ata_ix =
        create_associated_token_account(&payer.pubkey(), owner, &mint.pubkey(), &token_program);

    let mint_to_ix = mint_to(
        &token_program,
        &mint.pubkey(),
        &ata,
        &mint_authority.pubkey(),
        &[],
        amount,
    )
    .unwrap();

    send_multi(svm, payer, &[mint_authority], &[create_ata_ix, mint_to_ix]);

    mint.pubkey()
}

fn setup() -> (LiteSVM, Keypair, Keypair, Keypair) {
    let mut svm = LiteSVM::new();
    let program_id = escrow::id();
    let bytes = include_bytes!(concat!(
        env!("CARGO_TARGET_TMPDIR"),
        "/../deploy/escrow.so"
    ));
    svm.add_program(program_id, bytes).unwrap();

    let maker = Keypair::new();
    let taker = Keypair::new();
    let mint_authority = Keypair::new();

    svm.airdrop(&maker.pubkey(), 5_000_000_000).unwrap();
    svm.airdrop(&taker.pubkey(), 5_000_000_000).unwrap();
    svm.airdrop(&mint_authority.pubkey(), 5_000_000_000).unwrap();

    (svm, maker, taker, mint_authority)
}

#[test]
fn test_make() {
    let (mut svm, maker, taker, mint_authority) = setup();
    let program_id = escrow::id();

    let mint_a = create_mint_and_fund(
        &mut svm,
        &maker,
        &mint_authority,
        &maker.pubkey(),
        DEPOSIT_AMOUNT,
    );
    let mint_b = create_mint_and_fund(
        &mut svm,
        &maker,
        &mint_authority,
        &taker.pubkey(),
        RECEIVE_AMOUNT,
    );

    let (escrow_pda, _) = Pubkey::find_program_address(
        &[ESCROW_SEED, maker.pubkey().as_ref(), SEED.to_le_bytes().as_ref()],
        &program_id,
    );

    let token_program = spl_token_2022::id();
    let maker_ata_a =
        get_associated_token_address_with_program_id(&maker.pubkey(), &mint_a, &token_program);
    let vault =
        get_associated_token_address_with_program_id(&escrow_pda, &mint_a, &token_program);

    send(
        &mut svm,
        &maker,
        Instruction::new_with_bytes(
            program_id,
            &escrow::instruction::Make {
                seed: SEED,
                receive: RECEIVE_AMOUNT,
                deposit: DEPOSIT_AMOUNT,
            }
            .data(),
            escrow::accounts::Make {
                maker: maker.pubkey(),
                mint_a,
                mint_b,
                maker_ata_a,
                escrow: escrow_pda,
                vault,
                token_program,
                associated_token_program: anchor_spl::associated_token::ID,
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
        ),
    );

    assert!(svm.get_account(&escrow_pda).is_some(), "escrow should exist after make");
    assert!(svm.get_account(&vault).is_some(), "vault ATA should exist after make");
}

#[test]
fn test_take() {
    let (mut svm, maker, taker, mint_authority) = setup();
    let program_id = escrow::id();

    let mint_a = create_mint_and_fund(
        &mut svm,
        &maker,
        &mint_authority,
        &maker.pubkey(),
        DEPOSIT_AMOUNT,
    );
    let mint_b = create_mint_and_fund(
        &mut svm,
        &maker,
        &mint_authority,
        &taker.pubkey(),
        RECEIVE_AMOUNT,
    );

    let (escrow_pda, _) = Pubkey::find_program_address(
        &[ESCROW_SEED, maker.pubkey().as_ref(), SEED.to_le_bytes().as_ref()],
        &program_id,
    );

    let token_program = spl_token_2022::id();
    let maker_ata_a =
        get_associated_token_address_with_program_id(&maker.pubkey(), &mint_a, &token_program);
    let vault =
        get_associated_token_address_with_program_id(&escrow_pda, &mint_a, &token_program);

    // --- make ---
    send(
        &mut svm,
        &maker,
        Instruction::new_with_bytes(
            program_id,
            &escrow::instruction::Make {
                seed: SEED,
                receive: RECEIVE_AMOUNT,
                deposit: DEPOSIT_AMOUNT,
            }
            .data(),
            escrow::accounts::Make {
                maker: maker.pubkey(),
                mint_a,
                mint_b,
                maker_ata_a,
                escrow: escrow_pda,
                vault,
                token_program,
                associated_token_program: anchor_spl::associated_token::ID,
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
        ),
    );

    // --- take ---
    let taker_ata_a =
        get_associated_token_address_with_program_id(&taker.pubkey(), &mint_a, &token_program);
    let taker_ata_b =
        get_associated_token_address_with_program_id(&taker.pubkey(), &mint_b, &token_program);
    let maker_ata_b =
        get_associated_token_address_with_program_id(&maker.pubkey(), &mint_b, &token_program);

    send(
        &mut svm,
        &taker,
        Instruction::new_with_bytes(
            program_id,
            &escrow::instruction::Take {}.data(),
            escrow::accounts::Take {
                taker: taker.pubkey(),
                maker: maker.pubkey(),
                mint_a,
                mint_b,
                taker_ata_a,
                taker_ata_b,
                maker_ata_b,
                escrow: escrow_pda,
                vault,
                token_program,
                associated_token_program: anchor_spl::associated_token::ID,
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
        ),
    );

    assert!(svm.get_account(&escrow_pda).is_none(), "escrow should be closed after take");
    assert!(svm.get_account(&vault).is_none(), "vault should be closed after take");

    let taker_a_data = svm.get_account(&taker_ata_a).unwrap().data;
    let maker_b_data = svm.get_account(&maker_ata_b).unwrap().data;

    // Token-2022 account layout: amount is a u64 at byte offset 64
    let taker_a_amount = u64::from_le_bytes(taker_a_data[64..72].try_into().unwrap());
    let maker_b_amount = u64::from_le_bytes(maker_b_data[64..72].try_into().unwrap());

    assert_eq!(taker_a_amount, DEPOSIT_AMOUNT, "taker should receive mint_a");
    assert_eq!(maker_b_amount, RECEIVE_AMOUNT, "maker should receive mint_b");
}