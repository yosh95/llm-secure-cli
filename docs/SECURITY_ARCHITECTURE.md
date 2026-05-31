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

When `security_level` is set to `standard` (via `config.toml` or `LLM_CLI_SECURITY_LEVEL` env var), the system downgrades PQC enforcement from a "hard block" to a "logged warning," ensuring the agent can still function in mixed-trust environments while maintaining an audit trail of the unverified actions.

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
Simply run `llsc identity manifest` to generate the initial manifest.
Once the manifest is generated, all subsequent runs will strictly enforce
integrity against it.

---

## Distributed Zero-Trust (High-Assurance Mode)

In a distributed environment (e.g., Client Agent and Remote MCP Server),
the system shifts to a **Distributed Trust** model designed to eliminate
shared secrets:

1.  **Local Trust Store Model:** Keys are stored locally in `~/.llm_secure_cli/keys/`. Each entity maintains its own key pair with no sharing of private keys between entities.
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
| `id_pqc.key` / `id_pqc.pub` | `~/.llm_secure_cli/keys/` | ML-DSA-87 |
| `id_ed25519` / `id_ed25519.pub` | `~/.llm_secure_cli/keys/` | Ed25519 (classical) |
| `id_kem.key` / `id_kem.pub` | `~/.llm_secure_cli/keys/` | ML-KEM-1024 |

The application always uses ML-DSA-87 (FIPS 204 Level 5) for signing and ML-KEM-1024 (FIPS 203 Level 5) for encryption. There is no runtime variant selection — the highest NIST Level 5 is always used.

### Remote Attestation

On startup, the client generates a SHA-256 manifest of all critical security
source files and configuration files (defined by glob patterns in
`integrity.rs`), signed with an ML-DSA key. This manifest covers all Rust source files, configuration templates (`.toml`), and project metadata (e.g., `Cargo.toml`).
 Remote servers verify this
manifest to confirm the agent is running an authentic, unmodified stack.
The system also detects unauthorized new files that match the critical
patterns, preventing backdoor installation.

Rebuild the manifest after any code update or configuration change:

```bash
llsc identity manifest
```

### Audit Chain Continuity

- **Chained hashing** — each log entry includes a SHA-256 of the previous entry's hash, creating a tamper-evident chain.
- **Snapshot anchors** — on log rotation, a signed anchor entry records `snapshot_prev_hash` and `snapshot_first_hash` to maintain verifiability across file boundaries.
- **Merkle Tree anchoring** — a binary Merkle Root is computed over all entries on rotation and recorded in the security log.  This root can be submitted to an external immutable ledger (blockchain, transparency log) for public verification.
- **ML-KEM hybrid encryption** — audit logs are optionally encrypted with ML-KEM-1024 + AES-256-GCM to guarantee future quantum confidentiality. Decrypt with:

  ```bash
  llsc decrypt-log ~/.llm_secure_cli/logs/audit.jsonl -o decrypted.jsonl
  ```

**Audit log retention:** Back up `~/.llm_secure_cli/logs/audit.jsonl` and its rotated archives. Forward to a remote WORM store (SIEM) if available.

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
(overridden by `~/.llm_secure_cli/config.toml`):

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

# Pager: "" (disable), "auto" (try less/built-in), or a specific command
pager = "auto"

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

## Agent Skill Verification (Tier 4 — Supply-Chain Safety)

### Motivation

In December 2025, Anthropic released the [Agent Skills open
standard](https://agentskills.io/specification), a specification for
portable, cross-platform procedural knowledge packages (`SKILL.md`).  Within
90 days, 32+ AI coding tools adopted the format, and community marketplaces
accumulated tens of thousands of community-contributed skills.

This rapid, zero-friction ecosystem growth also created a **novel
supply-chain attack surface** that existing security infrastructure was not
designed to address.  Snyk's *ToxicSkills* study (February 2026)
documented:

| Finding | Scale |
|---|---|
| Skills with security flaws | 36% of 3,984 scanned |
| Confirmed malicious payloads | 76 skills |
| Coordinated campaign ("ClawHavoc") | 341 hostile skills delivering Atomic Stealer (AMOS) |
| Behavioral degradation from poor-quality skills | 6 measurable mechanisms (template propagation, pattern bleed, etc.) |

The attack surface is uniquely dangerous because Agent Skills execute inside
AI agents that already have shell access, file-system permissions, and
network connectivity.  A malicious npm package can access what Node.js
allows; a malicious skill inherits the **full permissions of the AI agent**
it runs inside.

`llsc` already addresses MCP security through the Verifier Committee and
PQC signatures.  The Agent Skill Verification feature (`llsc verify-skill`) extends
the same philosophy to the Skills layer.

### Architecture: The Three-Tier Verification Pipeline

The pipeline mirrors the Triple-Lock design of `llsc`'s main security
framework, adapted for static file analysis rather than live tool execution:

```
                      SKILL.md directory
                             │
              ┌──────────────┼──────────────┐
              ▼              ▼              ▼
        ┌──────────┐  ┌──────────┐  ┌──────────────┐
        │  Tier 1  │  │  Tier 2  │  │    Tier 3    │
        │ Structure│  │ Signature│  │   Semantic   │
        │          │  │          │  │   Firewall   │
        └────┬─────┘  └────┬─────┘  └──────┬───────┘
             │             │               │
             └─────────────┼───────────────┘
                           ▼
                    ┌──────────────┐
                    │   VERDICT    │
                    │ SAFE / SUSP /│
                    │  DANGEROUS   │
                    └──────────────┘
```

### Tier 1 — Structural Validation (src/security/skill_verifier.rs)

**Type**: Deterministic, sub-millisecond, no external dependencies.

Validates `SKILL.md` against the Agent Skills specification:

- YAML frontmatter delimited by `---`
- Required field `name`: 1–64 characters, `[a-z0-9-]` only
- Required field `description`: 1–1024 characters
- Directory structure: `SKILL.md` found (case-insensitive)
- Optional fields parsed but not required: `license`, `compatibility`,
  `metadata`, `allowed-tools`

A structural failure always results in a `DANGEROUS` verdict — the skill
does not conform to the standard and cannot be safely parsed by downstream
tools.

**Implementation**: A minimal hand-written YAML frontmatter parser
(`parse_frontmatter`) that avoids pulling in a full YAML library for a
two-field specification.  Handles `>-` folded block scalars for multi-line
descriptions.

### Tier 2 — Signature Verification (src/security/skill_verifier.rs)

**Type**: Cryptographic, local-only, no network dependency.

Checks for a `SKILL.md.sig` file alongside `SKILL.md`.  Two strategies are
attempted in order:

1. **COSE hybrid token** (Tag 98): The signature file is parsed as a COSE
   Sign structure.  The Ed25519 signature is verified against the project's
   `id_ed25519.pub`, and the ML-DSA signature against the appropriate
   variant's public key.  The `sub` claim from the token payload is
   extracted as the publisher identity.

2. **Raw Ed25519 fallback**: If COSE parsing fails, the signature file is
   treated as a 64-byte raw Ed25519 signature over the SKILL.md content,
   verified against `id_ed25519.pub`.

An unsigned skill is flagged `SUSPICIOUS` rather than `DANGEROUS` — the
absence of a signature is a trustworthiness concern, not a structural
defect.

**Future direction**: When the Agentic AI Foundation (AAIF) establishes a
verified-publisher PKI, this tier can be extended to verify signatures
against a trusted root rather than the project's own key.

### Tier 3 — Semantic Firewall (src/security/skill_verifier.rs)

**Type**: Probabilistic, LLM-dependent, requires configured Verifier
verifier.

This is the same Verifier Committee architecture used for tool-call verification,
repurposed with a **skill-specific security constitution**
(`SKILL_SECURITY_CONSTITUTION`).  The verifier LLM receives:

- The skill's declared `name` and `description` (from YAML frontmatter)
- The full `SKILL.md` body
- A hardcoded system prompt defining the threat model and judgment criteria

The verifier analyzes for:

| Threat Category | Detection Pattern |
|---|---|
| Hidden instructions | Content that contradicts or subverts the declared purpose |
| Data exfiltration | `curl`/`wget` to external hosts, reading secrets/env |
| Obfuscated payloads | base64, hex, or other encoded content |
| Social engineering | Instructions to manipulate the user through the agent |
| Security bypass | Attempts to disable or override security controls |

The response format is structured:

```
DECISION: [CLEAN | SUSPICIOUS | TOXIC]
CONFIDENCE: [0.0–1.0]
FINDINGS: [JSON array of per-finding category, description, confidence]
REASON: [One-sentence summary]
```

**Constitution isolation**: The `SKILL_SECURITY_CONSTITUTION` is a
hardcoded constant in `src/security/skill_verifier.rs`, separate from the
tool-call `SECURITY_CONSTITUTION` in `src/security/policy.rs`.  Each
constitution is purpose-built for its threat model, preventing prompt
leakage between concerns.

### Verdict Computation

```
structural failure ──────────────────────────► DANGEROUS
signature verification failure ──────────────► SUSPICIOUS
semantic TOXIC ──────────────────────────────► DANGEROUS
semantic SUSPICIOUS ─────────────────────────► SUSPICIOUS
semantic CLEAN + signed ─────────────────────► SAFE
semantic CLEAN + unsigned ───────────────────► SUSPICIOUS
no semantic analysis + signed ───────────────► SAFE
no semantic analysis + unsigned ─────────────► SUSPICIOUS
```

### Design Decisions

**"Verify, don't execute"** is the foundational principle.  `llsc` never
loads a skill into an agent's context window; it only audits the skill and
reports findings.  This creates a clean responsibility boundary: the tool
warns, the user decides.

**Semantic Firewall is opt-in** (`--semantic` flag).  Without it, Tiers 1
and 2 still catch structural failures and provide signature verification,
but a well-formed malicious skill will be rated `SUSPICIOUS` (unsigned)
rather than `DANGEROUS`.  This reflects the reality that deterministic
analysis cannot detect semantic malice — and that running an LLM-based
analysis has a non-zero cost (API call, latency).

**No execution sandbox for skills.**  This is intentional.  If a future
version adds skill execution, it will be gated behind:
1. Verified publisher signature (Tier 2)
2. Clean Semantic Firewall verdict (Tier 3)
3. Clean Semantic Firewall verdict (Tier 3)
4. Docker isolation by default

### Relationship to the Existing Triple-Lock Framework

Agent Skill Verification extends the Triple-Lock model with a fourth
dimension — **Supply-Chain Safety** — that applies the same Zero Trust
principles to content the agent *might* consume:

| Tier | Existing (Tool Execution) | New (Skill Verification) |
|---|---|---|
| T1 — Space | Path guardrails, static analysis | YAML structural validation |
| T2 — Behavior | Verifier Committee security verification, ABAC | Signature verification (Ed25519/ML-DSA) |
| T3 — Time | PQC audit trail, Merkle anchoring | Semantic Firewall for skill content |
| T4 — Supply Chain | — *(new)* | Three-tier skill safety audit |

### Implementation Files

| File | Role |
|---|---|
| `src/security/skill_verifier.rs` | Core logic: parser, validators, signature verification, Semantic Firewall integration, batch discovery |
| `src/cli/commands/skill_verify.rs` | CLI handler: report formatting, JSON output, batch summary |
| `src/main.rs` (`VerifySkill` command) | Subcommand registration and dispatch |
| `src/security/mod.rs` | Module registration |
