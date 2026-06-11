# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.4.x   | ✅ Active |
| < 0.4   | ❌ EOL    |

---

## Architecture Overview: Triple-Lock Framework

`llm-secure-cli` implements a **Triple-Lock** security framework across three
dimensions — Space, Behavior, and Time — designed for autonomous LLM agents
operating via the Model Context Protocol (MCP).  The Verifier Committee handles
all risk judgment — risk-level-based PQC variant switching has been discontinued.

```
 Agent Tool Request
        │
        ▼
  ┌──────────────────────────────┐
  │   Verifier Committee         │  ← N-member, any-flag policy
  │   (Semantic Firewall)        │
  └──────┬───────┬───────┬───────┘
         │       │       │
         ▼       ▼       ▼
      T1       T2       T3
    Space    Behav.    Time
  (Static   (Verifier  (PQC Audit/
   Anlys)    Comm.)    Merkle)
         │       │       │
         └───────┴───────┘
                 │
                 ▼
        Secure Tool Execution
```

---

## Tier 1 — Structural Guardrails (Space)

#### Static Analysis (Minimalist Fast-Fail)
A lightweight, deterministic check that blocks only **control characters and NULL bytes** which could destabilize the execution engine or corrupt audit logs. This is not a security boundary — it is a stability boundary. Complex intent judgment and risk assessment are entirely delegated to the Verifier Committee (Tier 2).

**Implementation**: `src/security/static_analyzer.rs`

#### Physical Isolation (Docker / WSL2)
`llm-secure-cli` is designed to be run within isolated environments.
- **Docker-native Posture**: Running the agent inside a Docker container provides a physical boundary between the AI and the host system. This makes the security posture uniform across Windows, Linux, and macOS by standardizing on a Linux container environment.

#### Path Guardrails (Verifier-based)
Path validation is handled entirely by the Verifier Committee. The static path whitelist (`allowed_paths`) has been removed — the verifier LLM uses its inherent knowledge of sensitive paths (like `C:\Windows` or `/etc`) together with the user's intent context to determine whether a file access is safe.

#### Environment Isolation (MCP)
High-risk tool execution is delegated to remote MCP servers running inside VMs or Docker containers (Shared-Nothing architecture).  Even if a generated script bypasses static analysis, any malicious activity is contained within a disposable, restricted environment with no access to the host's filesystem or credentials.

### Environment Isolation (MCP)

High-risk tool execution is delegated to remote MCP servers running inside
VMs or Docker containers (Shared-Nothing architecture).  Even if a generated
script bypasses static analysis, any malicious activity is contained within a
disposable, restricted environment with no access to the host's filesystem or
credentials.

**TCB note:** The server binary / container image and its launch configuration
are part of the Trusted Computing Base. Pin Docker image digests and enforce
SSH host-key verification.

---

## Tier 2 — Behavioral Zero-Trust (Behavior)

### Workload Identity — Hybrid COSE Tokens (RFC 9052)

Every MCP tool call is accompanied by a cryptographically signed identity
token encoded as a **COSE_Sign** structure (CBOR tag 98, RFC 9052).

**Native implementation:** The COSE layer is implemented directly using
Rust-native crates (`ed25519-dalek`, `ciborium`, `fips203`/`fips204`) — no external Python dependency. This makes custom algorithm identifiers (ML-DSA alg `−48`, IANA-pending per *draft-ietf-cose-dilithium*) fully auditable without registry injection.

**Token structure:**

```
COSE_Sign [CBOR tag 98]
├── body_protected   : cbor2.dumps({})
├── unprotected      : {}
├── payload          : cbor2.dumps(claims_dict)
└── signatures
    ├── [0] COSE_Signature   alg = -8  (Ed25519)
    │       protected   : cbor2.dumps({1: -8})
    │       unprotected : {}
    │       signature   : Ed25519 over Sig_Structure
    └── [1] COSE_Signature   alg = -48   (ML-DSA)
            protected   : cbor2.dumps({1: -48})
            unprotected : {4: b"ML-DSA-87"}   ← kid = variant name
            signature   : ML-DSA variant over Sig_Structure
```

**Sig_Structure (RFC 9052 §4.4):**

```python
cbor2.dumps(["Signature", body_protected, sign_protected, b"", payload])
```

**Key files:**
- `src/security/pqc_cose.rs` — Hybrid COSE token creation and verification
- `src/security/identity.rs` — `IdentityManager` key generation, token creation, and key management

**COSE algorithm constants:**

```rust
// Ed25519 (Classical signature)
// EdDSA with Curve25519 — RFC 8032 / COSE alg -8

// ML-DSA (Post-Quantum signature)
// ML-DSA — IANA pending: draft-ietf-cose-dilithium
// Algorithm identifier: -48 (provisional)

// COSE header parameter
// alg = 1 (RFC 9052 §3.1)
// COSE_Sign tag = 98
```

### AI-native ABAC (Policy Constitution)

`llm-secure-cli` utilizes an AI-native **Attribute-Based Access Control (ABAC)** model. Instead of maintaining thousand-line JSON/TOML rule-sets, the system gathers trusted context attributes and delegates the evaluation to a "Security Constitution".

**Attributes Gathered for Evaluation:**
- `os`: The operating system (e.g., "linux", "windows").
- `user`: The current system user.
- `current_dir`: The current working directory.
- `security_level`: The configured security level ("high" or "standard").
- `container_mode`: Whether Docker isolation is active.
- `is_git_repo`: Whether the session is inside a Git repository.

These attributes are bundled into a **Security Context** and verified by the Verifier Committee against the **Security Constitution** (a hardcoded, non-overridable policy set in the code). This allows for dynamic, context-aware decisions like "Allow deletions only if running inside a container" without complex manual configuration or OS-dependent static rules.

Implementation: `src/security/policy.rs` and `src/security/verifier.rs`.

Tool call risk classification is handled by the Verifier Committee — there is no separate
risk-level configuration file. The Verifier evaluates each tool call semantically
using the Security Constitution.

### Verifier Committee (Semantic Firewall — N-Member, Any-Flag Policy)

To prevent sophisticated Prompt Injection (especially indirect injection),
`llm-secure-cli` implements a **Verifier Committee** pattern with N independent
LLM members operating under an **"any-flag" policy**.

**How it works:**
1. ALL configured verifier members (including the primary `verifier_provider`/`verifier_model`) are consulted **concurrently**.
2. Each member receives only the **User's Original Prompt** and the **Proposed Tool Call** (excluding potentially tainted intermediate reasoning or large external data).
3. If ANY member flags the call as requiring review → human approval is mandatory.
4. Only if ALL members return ALLOW is the call auto-approved.
5. If ANY member is unavailable → FallbackRequired (human must decide).

| Feature | Implementation |
|---|---|
| Trigger | Every tool call (configurable via `verifier_enabled`) |
| Policy | "Any-flag" — one flag = human reviews, unanimous ALLOW = auto-approve |
| Isolation | Verifiers are stateless and have no tool access (function calling OFF) |
| Members | Primary: `verifier_provider` + `verifier_model` in `defaults.toml` |
|  | Additional: `[security.verifier_committee] members = [...]` |
| Verdict | Structured ALLOW/REVIEW/MODIFY with reason (BLOCK mapped to REVIEW) |
| Fallback | When any verifier is unavailable → always require human approval (`block` option removed) |

Implementation: `src/security/verifier.rs`.
Enable via `defaults.toml`: `verifier_enabled = true`.

### Verifier Fallback (Always Require Approval)

When any Verifier Committee member is unavailable (network error, API failure, etc.), the system always asks for human approval before executing the tool call. The previously configurable `block` policy has been removed — the system now consistently requires manual confirmation as the only fallback behavior. This applies to ALL committee members: if one member fails, the entire committee falls back to human review.

### Auto-Approval via Verifier Committee

The Verifier Committee determines which tool calls are auto-approved. If ALL committee
members return ALLOW (or MODIFY with corrections), the call is auto-approved. If ANY member
flags it as NeedsApproval, human approval is required. When the verifier is unavailable,
the system always falls back to human-in-the-loop (HITL) approval.

> **Note:** There is no configurable `auto_approval_level` setting. The Verifier Committee's
> semantic analysis is the sole determinant of whether a tool call is auto-approved.

### Compatibility & Interoperability (Security Levels)

To allow interoperability with standard MCP clients (e.g., Cursor, Claude Desktop) or third-party MCP servers that do not support PQC protocols, `llm-secure-cli` provides a configurable security level:

| Level | Enforcement | Use Case |
|---|---|---|
| **high** (Default) | Strict PQC checks. High-risk actions without signatures are blocked. | Enterprise / High-Assurance environments. |
| **standard** | Permissive checks. Warnings are logged but actions are permitted. | General use / Interoperability with third-party tools. |

When `security_level` is set to `standard` (via `config.toml`), the system downgrades PQC enforcement from a "hard block" to a "logged warning," ensuring the agent can still function in mixed-trust environments while maintaining an audit trail of the unverified actions.

### Bi-directional Verification (ResponseSigner)

High-risk write tools embed an ML-DSA signature in their return value,
binding the response to a unique `verification_id`.  The `tool_executor`
layer verifies this signature before passing the result to the LLM —
preventing Man-in-the-Middle manipulation of tool outputs.

Implementation: `PQCProvider.sign()` / `ResponseSigner.sign_response()` in
`src/security/pqc.rs`.

### Out-of-Band Key Distribution

Public keys and remote attestation manifests are distributed via an OOB
trusted channel (e.g., MDM, Secure Enclave, or enterprise PKI). This design
eliminates Trust-On-First-use (TOFU) in production deployments.

A bootstrap mode is available for standalone development environments.



---

## Distributed Zero-Trust (High-Assurance Mode)

In a distributed environment (e.g., Client Agent and Remote MCP Server),
the system shifts to a **Distributed Trust** model designed to eliminate
shared secrets:

1.  **Local Trust Store Model:** Keys are stored locally in `~/.llsc/keys/`. Each entity maintains its own key pair with no sharing of private keys between entities.
2.  **Impersonation Resistance:** The Agent ID is fixed to `user@hostname` (derived from `IdentityManager::get_local_identity()`). Removing the ability to override this ID prevents attackers with stolen keys from easily spoofing authorized identities on different hosts.
3.  **Automatic Key Generation:** Keys are automatically generated on first run when `IdentityManager::ensure_keys()` is called. However, security is enforced at the **Verification Layer** — any identity not backed by the local key store will fail verification.
4.  **Blast Radius Containment:** Because keys are not shared, the compromise of a remote MCP server does not expose the Agent's private identity key, preventing an attacker from impersonating the user in other contexts.
5.  **Mutual Authentication:** Every request/response loop is protected by mutual ML-DSA signatures. The Agent verifies the tool output's `ResponseSigner` signature, while the Server verifies the `IdentityToken` using the Agent's public key.

---

## Tier 3 — Post-Quantum Resilience (Time)

### PQC Primitives

| Algorithm | NIST FIPS | Security Level | Key / Sig Sizes | Default use |
|---|---|---|---|---|
| ML-DSA-87 | FIPS 204 | Level 5 | pk=2 592 B, sk=4 896 B, sig=4 595 B | All signing operations |
| ML-KEM-1024 | FIPS 203 | Level 5 | pk=1 568 B, sk=3 168 B, ct=1 568 B | All audit encryption |
| Ed25519 | RFC 8032 | 128-bit classical | pk=32 B, sk=32 B, sig=64 B | Identity tokens (pair with ML-DSA) |

Implementation: Rust-native `fips203`/`fips204` crates (ML-KEM/ML-DSA) — FIPS-compliant reference implementations.

### Classical Cryptography

| Algorithm | Purpose | Implementation |
|---|---|---|
| Ed25519 | Identity tokens, manifest signing | `ed25519-dalek` crate |

The hybrid token scheme uses **Ed25519** (classical) + **ML-DSA-87** (post-quantum) as the default signing pair, replacing the earlier RS256 designation.

### PQC at Maximum Strength

Risk-level-based PQC variant switching has been **discontinued**. All operations
use the highest available NIST Level 5 strength regardless of tool risk level:

| Algorithm | NIST FIPS | Security Level | Key / Sig Sizes | Use |
|---|---|---|---|---|
| ML-DSA-87 | FIPS 204 | Level 5 | pk=2 592 B, sk=4 896 B, sig=4 595 B | All signing operations |
| ML-KEM-1024 | FIPS 203 | Level 5 | pk=1 568 B, sk=3 168 B, ct=1 568 B | All audit encryption |

**Key files provisioned on disk:**

| File | Location | Algorithm |
|---|---|---|
| `id_pqc.key` / `id_pqc.pub` | `~/.llsc/keys/` | ML-DSA-87 |
| `id_ed25519` / `id_ed25519.pub` | `~/.llsc/keys/` | Ed25519 (classical) |
| `id_kem.key` / `id_kem.pub` | `~/.llsc/keys/` | ML-KEM-1024 |

The application always uses ML-DSA-87 (FIPS 204 Level 5) for signing and ML-KEM-1024 (FIPS 203 Level 5) for encryption. There is no runtime variant selection — the highest NIST Level 5 is always used.

### Remote Attestation (Planned)

Remote attestation is a future feature. It will allow a client to verify
that an agent process is running an authentic, unmodified stack through
cryptographic proofs.

### Audit Chain Continuity

- **Chained hashing** — each log entry includes a SHA-256 of the previous entry's hash, creating a tamper-evident chain.
- **Snapshot anchors** — on log rotation, a signed anchor entry records `snapshot_prev_hash` and `snapshot_first_hash` to maintain verifiability across file boundaries.
- **Merkle Tree anchoring** — a binary Merkle Root is computed over all entries on rotation and recorded in the security log.  This root can be submitted to an external immutable ledger (blockchain, transparency log) for public verification.
- **ML-KEM hybrid encryption** — audit logs are optionally encrypted with ML-KEM-1024 + AES-256-GCM to guarantee future quantum confidentiality. Decrypt with:

  ```bash
  llsc decrypt-log ~/.llsc/logs/audit.jsonl -o decrypted.jsonl
  ```

**Audit log retention:** Back up `~/.llsc/logs/audit.jsonl` and its rotated archives. Forward to a remote WORM store (SIEM) if available.

---

## Security Performance & Latency Benchmarks

To maintain an agile user experience, `llm-secure-cli` optimizes for minimal operational overhead. The following measurements were captured on reference hardware (AMD Ryzen 5, WSL2).

### 1. Cryptographic Latency (PQC)
Measured using the Rust implementation on reference hardware.

| Tier | Component | Algorithm | Avg Latency (ms) |
|---|---|---|---|
| Tier 1 | Minimalist Static Analysis | — | < 0.01 |
| Tier 2 | Identity Generation (Ed25519) | Ed25519 | < 0.1 |
| Tier 2 | Identity Generation | ML-DSA-87 | 1.26 |
| Tier 3 | Audit Encryption | ML-KEM-1024 | 0.09 |

### 2. Security Verification Latency (Verifier Committee)
Latency varies based on the provider and network conditions. We recommend lightweight "verifier" models to minimize the "Security Speed Bump."

The default verifier configuration uses `ollama` provider with `default` model. This can be overridden in `config.toml` via `verifier_provider` and `verifier_model`.

---

## Security Configuration Reference

The primary security configuration is in `src/config/defaults.toml`
(overridden by `~/.llsc/config.toml`):

```toml
[general]
pdf_as_base64 = true
request_timeout = 1800
command_timeout = 3600
image_save_path = "~/Pictures/llsc"
max_audit_log_lines = 10000
max_chat_log_lines = 5000
max_chat_archives = 5
max_output_lines = 5000
max_output_chars = 50000

[security]
# Security Level: "high" (Default) | "standard"
security_level = "high"

# Verifier Committee (AI-native ABAC / Semantic Firewall)
# N independent LLMs evaluate each tool call with an "any-flag" policy.
verifier_enabled = true
verifier_provider = "ollama"
verifier_model = "gemma4:e2b"

# Additional committee members (optional):
# [security.verifier_committee]
# members = [
#   { provider = "openai", model = "gpt-4o-mini" },
#   { provider = "openrouter", model = "anthropic/claude-3-haiku" },
# ]
```

---

## Dependency Security

| Package | Purpose | Notes |
|---|---|---|
| `ed25519-dalek` | Ed25519 signatures, key serialization | Rust-native; monitor upstream advisories |
| `fips203`/`fips204` | ML-KEM / ML-DSA | FIPS-compliant implementations |
| `ciborium` | CBOR serialization for COSE tokens | Rust-native CBOR implementation |
| `aes-gcm` | AES-256-GCM for audit log encryption | Used with ML-KEM for hybrid encryption |
| `sha2` | SHA-256 for integrity hashing | Standard cryptographic hash |

> **Protocol Stability:** The COSE layer is implemented using Rust-native crates that are fully auditable and stable across library versions.

---

## Known Limitations & Security Trade-offs

### 1. Minimalist Static Analysis
The current Tier 1 static analysis utilizes a **structural fast-fail mechanism** (`src/security/static_analyzer.rs`). It blocks:
- **Control characters and null bytes**: Characters that could disrupt the tool execution engine or log output.

Shell invocation pattern detection (`sh -c`, `bash -c`, etc.) has been **removed** as redundant — all semantic risk assessment is delegated to the Verifier Committee (Tier 2). The `Command::new` API already provides structural safety against shell injection by design.

While highly efficient (<0.01ms), this layer is intentionally minimal. **Real-world security relies on the Defense-in-Depth provided by Tier 2 (Verifier Committee) and Tier 3 (Audit Trail).** Complex intent analysis and semantic risk assessment are entirely delegated to the Verifier Committee.

### 2. Probabilistic Security Verification
Verifier Committee is a **probabilistic** defense. While it can achieve high accuracy with well-chosen verifier models, LLMs can hallucinate or fail to catch "jailbreak" style prompt injections. The Verifier Committee's semantic analysis — while highly effective — is probabilistic.  Choosing strong verifier models (e.g., Gemma-4, GPT-4o-mini) is important for balancing security and usability.

### 3. ML-DSA COSE algorithm identifier (`alg=−48`)

The ML-DSA algorithm identifier used in COSE tokens (`-48`) is provisional
(IANA pending per *draft-ietf-cose-dilithium*).  Until the IANA assignment
is finalized, interoperability with third-party COSE implementations that
use a different provisional identifier is not guaranteed.  `llsc` pins the
`fips203`/`fips204` crate versions to maintain consistency.

---