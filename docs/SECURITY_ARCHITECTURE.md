# Security Architecture

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.4.x   | ✅ Active |
| < 0.4   | ❌ EOL    |

---

## Architecture Overview: Triple-Lock Framework

`llm-secure-cli` implements a **Triple-Lock** security framework across three
dimensions — Space, Behavior, and Time — designed for autonomous LLM agents.
The Verifier Committee handles all risk judgment — PQC variants are configurable
via the `[pqc]` section in config.toml.

```
 Agent Tool Request
        │
        ▼
  ┌──────────────────────────────┐
  │   Verifier Committee         │  ← N-member, configurable policy
  │   (Semantic Firewall)        │     (majority / any-flag)
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

---

## Tier 2 — Behavioral Zero-Trust (Behavior)

### Workload Identity — Hybrid COSE Tokens (RFC 9052)

Every tool call is accompanied by a cryptographically signed identity
token encoded as a **COSE_Sign** structure (CBOR tag 98, RFC 9052).

**Native implementation:** The COSE layer is implemented directly using
Rust-native crates (`ed25519-dalek`, `ciborium`, `fips203`/`fips204`) — no external Python dependency.

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
    └── [1] COSE_Signature   alg = -85/-86/-87 (ML-DSA variant)
            protected   : cbor2.dumps({1: alg_id})
            unprotected : {}
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
// Algorithm identifiers (provisional):
//   ML-DSA-44: -85
//   ML-DSA-65: -86
//   ML-DSA-87: -87

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
- `security_level`: Fixed to "high" (hardcoded).
- `container_mode`: Whether Docker isolation is active.
- `is_git_repo`: Whether the session is inside a Git repository.

These attributes are bundled into a **Security Context** and verified by the Verifier Committee against the **Security Constitution** (a hardcoded, non-overridable policy set in the code).

Implementation: `src/security/policy.rs` and `src/security/verifier.rs`.

### Verifier Committee (Semantic Firewall — N-Member, Configurable Policy)

To prevent sophisticated Prompt Injection (especially indirect injection),
`llm-secure-cli` implements a **Verifier Committee** pattern with N independent
LLM members operating under a configurable voting policy.

**How it works:**
1. ALL configured verifier members (managed via `/verifier add|delete`, persisted to state.toml) are consulted **concurrently**.
2. Each member receives only the **User's Original Prompt** and the **Proposed Tool Call** (excluding potentially tainted intermediate reasoning or large external data).
3. Verdicts are aggregated according to the configured **committee_policy**:
   - **`majority`** (default): Majority vote decides. Reduces unnecessary human review.
   - **`any-flag`** (conservative): If ANY member flags → human review mandatory.
4. If ANY member is unavailable → FallbackRequired (human must decide).

| Feature | Implementation |
|---|---|
| Trigger | Every tool call (configurable via `verifier_enabled`) |
| Policy | `"majority"` (default) or `"any-flag"` — set via `committee_policy` in config.toml |
| Isolation | Verifiers are stateless and have no tool access (function calling OFF) |
| Members | Runtime-managed via `/verifier add|delete` → persisted in state.toml |
|  | Fallback: `security.verifier_committee` in config.toml |
| Verdict | Structured ALLOW/REVIEW/MODIFY with reason |
| Fallback | When any verifier is unavailable → always require human approval |

Implementation: `src/security/verifier.rs`.
Enable via `config.toml`: `verifier_enabled = true`.

### Committee Voting Policy

Configured via `committee_policy` in the `[security]` section of config.toml:

| Policy | Description | Best For |
|--------|-------------|----------|
| `"majority"` (default) | Simple majority vote decides. Ties default to human review. | Balanced security & usability |
| `"any-flag"` | ANY member flags → human approval required. ALL allow → auto-approve. | Maximum security (minimizes missed attacks) |

### Verifier Fallback (Always Require Approval)

When any Verifier Committee member is unavailable (network error, API failure, etc.), the system always asks for human approval before executing the tool call. This applies to ALL committee members: if one member fails, the entire committee falls back to human review.

### Bi-directional Verification (ResponseSigner)

High-risk write tools embed an ML-DSA signature in their return value,
binding the response to a unique `verification_id`. The `tool_executor`
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

In a distributed environment (e.g., Client Agent and Remote Server),
the system shifts to a **Distributed Trust** model designed to eliminate
shared secrets:

1. **Local Trust Store Model:** Keys are stored locally in `~/.llsc/keys/self/me/`. Each entity maintains its own key pair with no sharing of private keys between entities.
2. **Impersonation Resistance:** The Agent ID is fixed to `user@hostname` (derived from `IdentityManager::get_local_identity()`). Removing the ability to override this ID prevents attackers with stolen keys from easily spoofing authorized identities on different hosts.
3. **Automatic Key Generation:** Keys are automatically generated on first run when `IdentityManager::ensure_keys()` is called. However, security is enforced at the **Verification Layer** — any identity not backed by the local key store will fail verification.
4. **Blast Radius Containment:** Because keys are not shared, the compromise of a remote server does not expose the Agent's private identity key, preventing an attacker from impersonating the user in other contexts.
5. **Mutual Authentication:** Every request/response loop is protected by mutual ML-DSA signatures. The Agent verifies the tool output's `ResponseSigner` signature, while the Server verifies the `IdentityToken` using the Agent's public key.

---

## Tier 3 — Post-Quantum Resilience (Time)

### PQC Primitives

| Algorithm | NIST FIPS | Security Level | Key / Sig Sizes | Default Variant |
|---|---|---|---|---|
| ML-DSA-44 | FIPS 204 | Level 2 | pk=1 312 B, sk=2 560 B, sig=2 420 B | Default (signing) |
| ML-DSA-65 | FIPS 204 | Level 3 | pk=1 952 B, sk=4 032 B, sig=3 309 B | Optional |
| ML-DSA-87 | FIPS 204 | Level 5 | pk=2 592 B, sk=4 896 B, sig=4 595 B | Optional (max) |
| ML-KEM-512 | FIPS 203 | Level 1 | pk=800 B, sk=1 632 B, ct=768 B | Default (KEM) |
| ML-KEM-768 | FIPS 203 | Level 3 | pk=1 184 B, sk=2 400 B, ct=1 088 B | Optional |
| ML-KEM-1024 | FIPS 203 | Level 5 | pk=1 568 B, sk=3 168 B, ct=1 568 B | Optional (max) |
| Ed25519 | RFC 8032 | 128-bit classical | pk=32 B, sk=32 B, sig=64 B | Identity tokens (paired with ML-DSA) |

Implementation: Rust-native `fips203`/`fips204` crates (ML-KEM/ML-DSA) — FIPS-compliant reference implementations.

### Configurable PQC Variants

PQC variants are configurable via the `[pqc]` section in `~/.llsc/config.toml`:

```toml
[pqc]
# Signature variant: "ml-dsa-44" (default, Level 2), "ml-dsa-65" (Level 3), or "ml-dsa-87" (Level 5)
signature_variant = "ml-dsa-44"

# KEM variant: "ml-kem-512" (default, Level 1), "ml-kem-768" (Level 3), or "ml-kem-1024" (Level 5)
kem_variant = "ml-kem-512"
```

The default configuration uses the lowest variants (ML-DSA-44 / ML-KEM-512) for
performance. Upgrade to higher variants for increased security assurance.

### Classical Cryptography

| Algorithm | Purpose | Implementation |
|---|---|---|
| Ed25519 | Identity tokens, manifest signing | `ed25519-dalek` crate |

The hybrid token scheme uses **Ed25519** (classical) + **ML-DSA** (post-quantum) as the default signing pair.

### Key Files Provisioned on Disk

Key files are stored under `~/.llsc/keys/self/me/`:

| File | Algorithm |
|---|---|
| `id_ed25519` / `id_ed25519.pub` | Ed25519 (classical) |
| `id_mldsa44` / `id_mldsa44.pub` | ML-DSA-44 (default) |
| `id_kem512` / `id_kem512.pub` | ML-KEM-512 (default) |

> **Note:** If a different variant is configured (e.g., ML-DSA-87), the
> corresponding key files (`id_mldsa87` / `id_mldsa87.pub`) will be generated
> on the next `llsc keygen` run. You may need to regenerate keys after changing
> the variant.

### Remote Attestation (Planned)

Remote attestation is a future feature. It will allow a client to verify
that an agent process is running an authentic, unmodified stack through
cryptographic proofs.

### Audit Chain Continuity

- **Chained hashing** — each log entry includes a SHA-256 of the previous entry's hash, creating a tamper-evident chain.
- **Snapshot anchors** — on log rotation, a signed anchor entry records `snapshot_prev_hash` and `snapshot_first_hash` to maintain verifiability across file boundaries.
- **Merkle Tree anchoring** — a binary Merkle Root is computed over all entries on rotation and recorded in the security log. This root can be submitted to an external immutable ledger (blockchain, transparency log) for public verification.
- **ML-KEM hybrid encryption** — audit logs are optionally encrypted with ML-KEM + AES-256-GCM to guarantee future quantum confidentiality. Decrypt with:

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
| Tier 2 | Identity Generation (ML-DSA-44) | ML-DSA-44 | ~0.3 |
| Tier 3 | Audit Encryption (ML-KEM-512) | ML-KEM-512 | ~0.05 |

### 2. Security Verification Latency (Verifier Committee)
Latency varies based on the provider and network conditions. We recommend lightweight "verifier" models to minimize the "Security Speed Bump."

The default verifier configuration uses members configured via `/verifier add` or
the `verifier_committee` fallback in config.toml.

---

## Configuration Reference

The primary security configuration is in `src/config/defaults.toml`
(overridden by `~/.llsc/config.toml`):

```toml
[general]
pdf_as_base64 = true
request_timeout = 300
command_timeout = 300
image_save_path = "~/Pictures/llsc"
max_audit_log_lines = 10000
max_chat_log_lines = 5000
max_chat_archives = 5
max_output_lines = 5000
max_output_chars = 50000

[security]
auto_approve = false
verifier_enabled = true

# Verifier Committee members (fallback when state.toml has no runtime members):
# verifier_committee = ["ollama:gemma4:e2b", "openai:gpt-4o-mini"]

# Voting policy: "majority" (default) or "any-flag"
# committee_policy = "majority"

[pqc]
# Signature variant: "ml-dsa-44" (default, Level 2), "ml-dsa-65", "ml-dsa-87"
# signature_variant = "ml-dsa-44"

# KEM variant: "ml-kem-512" (default, Level 1), "ml-kem-768", "ml-kem-1024"
# kem_variant = "ml-kem-512"
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

---

## Known Limitations & Security Trade-offs

### 1. Minimalist Static Analysis
The current Tier 1 static analysis utilizes a **structural fast-fail mechanism** (`src/security/static_analyzer.rs`). It blocks:
- **Control characters and null bytes**: Characters that could disrupt the tool execution engine or log output.

Shell invocation pattern detection (`sh -c`, `bash -c`, etc.) has been **removed** as redundant — all semantic risk assessment is delegated to the Verifier Committee (Tier 2). The `Command::new` API already provides structural safety against shell injection by design.

While highly efficient (<0.01ms), this layer is intentionally minimal. **Real-world security relies on the Defense-in-Depth provided by Tier 2 (Verifier Committee) and Tier 3 (Audit Trail).**

### 2. Probabilistic Security Verification
The Verifier Committee is a **probabilistic** defense. While it can achieve high accuracy with well-chosen verifier models, LLMs can hallucinate or fail to catch "jailbreak" style prompt injections. Choosing strong verifier models (e.g., Gemma-4, GPT-4o-mini) is important for balancing security and usability.

### 3. ML-DSA COSE Algorithm Identifiers

The ML-DSA algorithm identifiers used in COSE tokens (`-85`, `-86`, `-87`) are
provisional (IANA pending per *draft-ietf-cose-dilithium*). Until the IANA assignment
is finalized, interoperability with third-party COSE implementations that
use a different provisional identifier is not guaranteed. `llsc` pins the
`fips203`/`fips204` crate versions to maintain consistency.

### 4. Hardcoded security_level
The `security_level` field in `SecurityContext` is always set to `"high"` in
the code (`src/security/policy.rs`). Unlike what earlier documentation suggested,
this value is not configurable via config.toml. The actual PQC variant selection
is controlled independently via the `[pqc]` section.

---

## Verifier LLM Benchmark Results

See [benchmark_results.md](./benchmark_results.md) for detailed verifier model
comparisons across 87 test scenarios (34 SAFE / 53 BLOCKED). Key findings:

| Model | Accuracy | Precision | Recall | F1 | Avg Latency |
|-------|----------|-----------|--------|-----|-------------|
| 🥇 **google/gemma-4-26b-a4b-it** | **100.00%** | **1.0000** | **1.0000** | **1.0000** | 2,144 ms |
| 🥈 openai/gpt-oss-120b:nitro | **95.40%** | **1.0000** | 0.9245 | **0.9608** | **681 ms** |
