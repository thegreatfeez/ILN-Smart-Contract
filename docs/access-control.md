# Access Control Matrix

## 1. Overview

The ILN-Smart-Contract implements a centralized access-control architecture to guarantee that all protocol operations are properly authorized. By centralizing permissions into shared guards, we achieve:
- **Consistency**: All similar checks behave exactly the same way across different endpoints.
- **Audibility**: Clear, easily reviewable access annotations on every public instruction.
- **Maintainability**: Reduced code duplication by eliminating inline authorization checks.

Security goals include enforcing the principle of least privilege, preventing unauthorized state mutations, and ensuring that any authorization failure immediately returns a deterministic contract error.

## 2. Role Definitions

### Submitter
Represents a freelancer or service provider who submits invoices to the protocol.
- **Can**: Create invoices, update invoices before funding, cancel un-funded invoices, and transfer invoice ownership.
- **Cannot**: Modify another user's invoice, force funding, or alter protocol configuration.

### Payer
The client who owes payment on the submitted invoice.
- **Can**: Pay the invoice (mark paid), file an appeal if a default occurs unfairly.
- **Cannot**: Create an invoice on behalf of a submitter, modify invoice terms, or claim yields.

### LP (Liquidity Provider)
Entities providing liquidity to fund pending invoices.
- **Can**: Join funding queues, fund approved invoices, claim yields, and claim default refunds.
- **Cannot**: Approve themselves without queue resolution, modify invoice terms, or appeal a default.

### Admin
The protocol administrator.
- **Can**: Update fee rates, maximum discount rates, distribution contracts, manage allowed tokens, pause/unpause the protocol, and resolve default appeals.
- **Cannot**: Arbitrarily modify invoice ownership, submit invoices as users without explicit authorization, or drain funds.

### Governance
Reserved for future DAO or multisig control over core parameter changes. Currently delegates to Admin functionality.

### Anyone
Publicly accessible read or state-transition functions that do not require specific authorization.
- **Can**: Read contract stats, query scores, resolve fund queues, and expire timed-out invoices.

## 3. Instruction Permission Matrix

| Instruction | Allowed Role(s) | Description |
| ----------- | --------------- | ----------- |
| `initialize` | Anyone | Initializes the contract once |
| `set_admin` | Admin | Updates the contract administrator address |
| `update_fee_rate` | Admin | Sets the protocol fee rate |
| `update_max_discount` | Admin | Updates the maximum allowed discount rate |
| `set_distribution_contract`| Admin | Updates the distribution contract address |
| `add_token` | Admin | Adds a supported token to the protocol |
| `remove_token` | Admin | Removes a supported token |
| `pause` | Admin | Pauses the protocol for emergency |
| `unpause` | Admin | Resumes protocol operations |
| `get_contract_stats` | Anyone | Reads protocol statistics |
| `submit_invoice` | Submitter | Submits a new invoice |
| `update_invoice` | Submitter | Updates an existing un-funded invoice |
| `submit_invoices_batch` | Submitter | Submits multiple invoices |
| `join_fund_queue` | LP | Enqueues intent to fund an invoice |
| `resolve_fund_queue` | Anyone | Selects the LP with highest reputation |
| `fund_invoice` | LP | Funds a pending invoice |
| `transfer_invoice` | Submitter | Transfers ownership of an invoice |
| `cancel_invoice` | Submitter | Cancels an un-funded invoice |
| `expire_invoice` | Anyone | Marks a pending expired invoice as Expired |
| `mark_paid` | Payer | Pays off an invoice |
| `claim_yield` | LP | Claims yield for a paid invoice |
| `claim_default` | LP | Claims refund for a defaulted invoice |
| `appeal_default` | Payer | Appeals an unfair default |
| `resolve_appeal` | Admin | Approves or rejects a default appeal |
| `payer_score` | Anyone | Reads a payer's reputation score |
| `lp_score` | Anyone | Reads an LP's reputation score |
| `suggested_discount_rate` | Anyone | Calculates discount rate based on score |
| `get_invoice` | Anyone | Reads invoice details |
| `get_invoice_count` | Anyone | Reads total invoice count |

## 4. Security Notes

- **Principle of Least Privilege**: Each instruction relies only on the minimal authority required to execute.
- **Centralized Verification**: Extracted inline logic ensures uniform verification logic and robust testing.
- **Auditability Improvements**: Every guard clearly emits a deterministic `Unauthorized` error instead of panicking, enhancing tracing.
- **Rejection Behavior**: If authorization fails, the protocol safely rejects the mutation without consuming extra gas or altering contract state.
