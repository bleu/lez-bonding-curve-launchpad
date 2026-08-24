# GTM-521 private-buy atomicity validation

## Result

**Blocked pending a validated SDK implementation on pinned LEZ v0.2.0.** The
current wallet facades expose custom-token collateral and native-gas deshields
as separate submissions, while this repository has not implemented and
acceptance-tested an atomic composite alternative. The SDK and CLI therefore
fail closed; they do not submit a weaker multi-transaction flow.

## Evidence

The pinned LEZ wallet starts a privacy-preserving transaction from one
`instruction_data` value and one `ProgramWithDependencies` in
`WalletCore::send_privacy_preserving_tx_with_pre_check`. That program bundle can
declare dependencies, and the privacy circuit executes chained calls from its
program output in the same proof/transaction.

The two required deshields are separate wallet facade operations:

- `Token::send_transfer_transaction_deshielded` builds a token-program
  `Instruction::Transfer` and submits it through that one-program boundary.
- `NativeTokenTransfer::send_deshielded_transfer` separately builds the native
  transfer program instruction and submits it through the same boundary.

Neither facade accepts a second instruction or program, so combining those
existing calls would produce two transactions and expose a funded transient
account between them, violating GTM-521's indivisible collateral-and-gas
requirement. This demonstrates a missing SDK composition, not a limitation of
the underlying chained-call transaction model.

## Repository behavior

`launchpad private-buy` accepts the intended private source and explicit gas
reserve parameters, but `launchpad-client::validate_private_buy_request` rejects
the operation before wallet access or submission. A zero gas reserve is rejected
separately. The command deliberately has no resume path: no funded operation can
be created until the atomic initial leg is implemented and validated.

The regression tests are:

```sh
cargo test -p launchpad-client private_buy
cargo test -p launchpad-cli private_buy
```

## Unblock condition

Reopen implementation when an SDK-owned composite program and dependency
manifest can execute both custom-token and native deshield legs atomically on
the pinned release, and prove it with local-sequencer acceptance tests. At that
point, replace the fail-closed gate with the SDK-owned deshield → public
exact-output buy → re-shield lifecycle and its local-sequencer acceptance tests.
