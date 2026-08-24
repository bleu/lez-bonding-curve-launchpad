# GTM-521 private-buy atomicity validation

## Result

**Blocked by the pinned LEZ v0.2.0 transaction model.** The required initial
operation cannot atomically deshield custom-token collateral and native gas into
one transient public account. The SDK and CLI therefore fail closed; they do not
submit a weaker multi-transaction flow.

## Evidence

The pinned LEZ wallet builds a privacy-preserving transaction from exactly one
`instruction_data` value and one `ProgramWithDependencies` in
`WalletCore::send_privacy_preserving_tx_with_pre_check`.

The two required deshields are separate wallet facade operations:

- `Token::send_transfer_transaction_deshielded` builds a token-program
  `Instruction::Transfer` and submits it through that one-program boundary.
- `NativeTokenTransfer::send_deshielded_transfer` separately builds the native
  transfer program instruction and submits it through the same boundary.

Neither facade accepts a second instruction or program, and the transaction
message contains one proved execution. Combining those calls would produce two
transactions and expose a funded transient account between them, violating
GTM-521's indivisible collateral-and-gas requirement.

## Repository behavior

`launchpad private-buy` accepts the intended private source and explicit gas
reserve parameters, but `launchpad-client::validate_private_buy_request` rejects
the operation before wallet access or submission. A zero gas reserve is rejected
separately. The command deliberately has no resume path: no funded operation can
be created while the atomic initial leg is unavailable.

The regression tests are:

```sh
cargo test -p launchpad-client private_buy
cargo test -p launchpad-cli private_buy
```

## Unblock condition

Reopen implementation only when the LEZ pin exposes a single
privacy-preserving transaction that can execute both the custom-token and native
deshield legs atomically (or an equivalent atomic composite program with the
same proof guarantees). At that point, replace the fail-closed gate with the
SDK-owned deshield → public exact-output buy → re-shield lifecycle and its
local-sequencer acceptance tests.
