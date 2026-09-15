# Permissionless namespaces within one curve deployment

[Logos PR #204](https://github.com/logos-co/rfp/pull/204) replaces deployment-wide administration with independent launchpad namespaces. Each namespace is identified by the public key that signs its initialization, with a separate config PDA; the initializer may nominate another admin. This prevents taking over an uninitialized identity while allowing anyone to establish a namespace without the deployer's permission.

Namespace identity is immutable and distinct from the current admin. The config seed is the identity's 32 bytes; pool seeds hash the ordered token pair, owner, and namespace. Factory seeds hash a padded type tag, sale salt, and namespace. Creator commitments also include the namespace. Reserve ATAs inherit their namespace through their pool owner. Every client path, including the private router's account list, uses these derivations.

This supersedes ADR 0003's singleton and compiled genesis authority. Config replacement remains an admin-signed operation with single-step transfer; explicit renunciation permanently sets the admin to the default key and prevents reinitialization. Rates remain live at swap execution; the existing collateral fee math and slippage checks stay unchanged. The external RFP-001 library remains a documented integration seam.

The account and wire layout changes require a fresh PoC deployment. Keeping a legacy singleton fallback would preserve the global authority that the new requirements remove, so no fallback is provided. Tests exercise independent fees and treasuries, mixed-account rejection, transfer/renunciation, and fee updates between quote and execution. They do not claim live-sequencer or production-proving validation.
