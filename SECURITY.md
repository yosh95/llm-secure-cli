# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.2.x   | OK Active  |
| < 0.2   | ❌ EOL     |

---

## Architecture Overview: Triple-Lock Framework

`llm-secure-cli` implements a **Triple-Lock** security framework across three
dimensions — Space, Behavior, and Time — designed for autonomous LLM agents
operating via the Model Context Protocol (MCP).  The central orchestration
engine is **CASS (Context-Adaptive Security Scaling)**, which dynamically
adjusts security posture based on each tool call's risk profile.

```
 Agent Tool Request
        │
        ▼
  ┌─────────────┐
  │    CASS     │  ← Context-Adaptive Security Scaling
  └──┬──┬──┬───┘
     │  │  │
     ▼  ▼  ▼
  T1  T2  T3
Space Beh Time
  │   │   │
  └───┴───┘
       │
       ▼
  Secure Tool Execution
```

---

## Tier 1 — Structural Guardrails (Space)

### AI-native Policy Engine (Semantic Guardrails)

Instead of fragile, platform-dependent regex patterns (e.g., `rm -rf /` or Windows-specific commands), `llm-secure-cli` uses an **AI-native Policy Engine**.
- **Security Constitution**: A hardcoded, immutable system instruction that the auditor LLM must follow.
- **Context Injection**: Attributes like OS, User, and Current Directory are injected into every verification request.
- **Semantic Analysis**: The auditor understands the *impact* of a command, catching novel or obfuscated attacks that would bypass static analysis.

### Physical Isolation (Docker / WSL2)

`llm-secure-cli` is designed to be run within isolated environments.
- **Docker-native Posture**: Running the agent inside a Docker container provides a physical boundary between the AI and the host system. This makes the security posture uniform across Windows, Linux, and macOS by standardizing on a Linux container environment.

### Path Guardrails (Simplified)

Paths are normalized to absolute form and checked against a basic whitelist (`allowed_paths`). Complex OS-specific blacklists are deprecated in favor of the Semantic Policy Engine, which recognizes sensitive paths like `C:\Windows` or `/etc` based on its inherent knowledge and the provided context.

### Resource Limits

In the modern architecture, hard resource enforcement (Memory, CPU) is offloaded to the **Isolation Layer** (e.g., Docker flags `--memory`, `--cpus`). Code-level `rlimit` is kept as a stub to maintain cross-platform compatibility without `libc` dependencies. Output length is still enforced by `tool_executor.rs` to prevent Denial-of-Wallet attacks.

### Environment Isolation (MCP)

High-risk tool execution is delegated to remote MCP servers running inside
VMs or Docker containers (Shared-Nothing architecture).  Even if a generated
script bypasses static analysis, any malicious activity is contained within a
disposable, restricted environment with no access to the host's filesystem or
credentials.

**Least-privilege MCP server configuration:**

```toml
[[mcp_servers]]
name   = "ops"
command = "ssh"
args   = ["user@host", "llsc", "--mcp-server"]
roles  = ["user"]   # never default to "admin"

[[mcp_servers]]
name    = "github"
command = "docker"
args    = ["run", "--rm", "-i", "--network=none",
           "ghcr.io/github/github-mcp-server:latest"]
roles   = ["guest"]
```

**TCB note:** The server binary / container image and its launch
configuration are part of the Trusted Computing Base.  Pin Docker image
digests and enforce SSH host-key verification.

---

## Tier 2 — Behavioral Zero-Trust (Behavior)

### Workload Identity — Hybrid COSE Tokens (RFC 9052)

Every MCP tool call is accompanied by a cryptographically signed identity
token encoded as a **COSE\_Sign** structure (CBOR tag 98, RFC 9052).

**Native implementation:** The COSE layer is implemented directly using
Rust-native crates (e.g., `ring`, `ciborium`) — no `pycose` dependency.  This makes custom
algorithm identifiers (ML-DSA alg `−48`, IANA-pending per
*draft-ietf-cose-dilithium*) fully auditable without registry injection.

**Token structure:**

```
COSE_Sign [CBOR tag 98]
├── body_protected   : cbor2.dumps({})
├── unprotected      : {}
├── payload          : cbor2.dumps(claims_dict)
└── signatures
    ├── [0] COSE_Signature   alg = -257  (RS256)
    │       protected   : cbor2.dumps({1: -257})
    │       unprotected : {}
    │       signature   : RSA-PKCS1v15/SHA-256 over Sig_Structure
    └── [1] COSE_Signature   alg = -48   (ML-DSA)
            protected   : cbor2.dumps({1: -48})
            unprotected : {4: b"ML-DSA-65"}   ← kid = variant name (agility)
            signature   : ML-DSA variant over Sig_Structure
```

**Sig\_Structure (RFC 9052 §4.4):**

```python
cbor2.dumps(["Signature", body_protected, sign_protected, b"", payload])
```

**Key files:**
- `src/security/pqc.rs` — `HybridSigner.create_hybrid_token()` /
  `verify_hybrid_token()`
- `src/security/identity.rs` — `IdentityManager.generate_token()` /
  `verify_token()`

**COSE algorithm constants:**

```python
_COSE_ALG_RS256  = -257   # RSASSA-PKCS1-v1_5 + SHA-256  (RFC 9052 §9.1)
_COSE_ALG_MLDSA  = -48    # ML-DSA (IANA pending: draft-ietf-cose-dilithium)
_COSE_HEADER_ALG =  1     # RFC 9052 §3.1
_COSE_SIGN_TAG   = 98     # CBOR tag for COSE_Sign
```

### AI-native ABAC (Policy Constitution)

`llm-secure-cli` utilizes an AI-native **Attribute-Based Access Control (ABAC)** model. Instead of maintaining thousand-line JSON/TOML rule-sets, the system gathers trusted context attributes and delegates the evaluation to a "Security Constitution".

**Attributes Gathered for Evaluation:**
- `os`: The operating system (e.g., "linux", "windows").
- `user`: The current system user.
- `current_dir`: The current working directory.
- `container_mode`: Whether Docker isolation is active.
- `is_git_repo`: Whether the session is inside a Git repository.

These attributes are bundled into a **Security Context** and verified by the Dual LLM against the **Security Constitution** (a hardcoded, non-overridable policy set in the code). This allows for dynamic, context-aware decisions like "Allow deletions only if running inside a container" without complex manual configuration.

Implementation: `src/security/abac.rs` and `src/security/policy.rs`.

Risk-level classification in `defaults.toml`:

```toml
[security]
high_risk_tools   = ["execute_command", "edit_file", "create_or_overwrite_file", "read_url_content", "brave_search"]
medium_risk_tools = ["read_file_content", "grep_files"]
# Low-risk tools → list_files_in_directory, search_files
```

Implementation: `src/security/policy.rs`.

### Dual LLM Verification (Dynamic Intent Check)

To prevent sophisticated Prompt Injection (especially indirect injection),
`llm-secure-cli` implements a **Dual LLM Verification** pattern. When a high-risk tool
is requested, the system intercepts the execution and consults a separate,
lightweight "Verifier" model.

The Verifier is provided only with the **User's Original Prompt** and the
**Proposed Tool Call** (excluding potentially tainted intermediate reasoning
or large external data). It must confirm that the action aligns with the
user's intent.

| Feature | Implementation |
|---|---|
| Trigger | High-risk tools (e.g., `execute_command`, `edit_file`) |
| Isolation | Verifier is stateless and has no tool access (function calling OFF) |
| Models | Lightweight, non-reasoning (e.g., `gemini-3.1-flash-lite`, `gpt-5.4-nano`) |
| Outcome | Safe (continue) / Unsafe (block with reason) |

Implementation: `src/security/dual_llm_verifier.rs`.
Enable via `defaults.toml`: `dual_llm_verification = true`.

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
eliminates Trust-On-First-Use (TOFU) in production deployments.

A bootstrap mode is available for standalone development environments. 
Simply run `llm-secure-cli-security manifest` to generate the initial manifest.
Once the manifest is generated, all subsequent runs will strictly enforce
integrity against it.

---

## Distributed Zero-Trust (High-Assurance Mode)

In a distributed environment (e.g., Client Agent and Remote MCP Server),
the system shifts to a **Distributed Trust** model designed to eliminate
shared secrets:

1.  **Trusted Directory Model:** Servers store Agent public keys in
    `~/.src/trusted/<entity_id>/id_pqc_<level>.pub`. There is no sharing
    of private keys between entities.
2.  **Impersonation Resistance:** The Agent ID is fixed to `user@hostname`.
    Removing the ability to override this ID prevents attackers with stolen
    keys from easily spoofing authorized identities on different hosts.
3.  **Automatic Key Generation:** For better UX, keys are automatically
    generated on first run. However, security is enforced at the **Verification
    Layer** — servers will reject any key not explicitly provisioned in their
    `trusted/` directory.
4.  **Blast Radius Containment:** Because keys are not shared, the compromise
    of a remote MCP server does not expose the Agent's private identity key,
    preventing an attacker from impersonating the user in other contexts.
5.  **Mutual Authentication:** Every request/response loop is protected by
    mutual ML-DSA signatures. The Agent verifies the tool output's
    `ResponseSigner` signature using the Server's public key (if trusted),
    while the Server verifies the `IdentityToken` using the Agent's public key.

### Trust Resolution & KMS Integration (Enterprise)

To support large-scale deployments, `llm-secure-cli` abstracts key resolution
through the `TrustResolver` interface (`src/security/trust.rs`). This
allows for seamless transition between local and enterprise trust models:

| Resolver | Storage Mechanism | Use Case |
|---|---|---|
| `LocalTrustResolver` | `~/.src/trusted/` | Standard / Decentralized |
| `KMSTrustResolver` | Remote KMS API / HSM | Enterprise / Centralized |

### Hardware Sovereignty (TEE-protected PQC)

For environments requiring high-assurance protection of private keys,
the **TEEPQCBackend** (`src/security/tee_backend.rs`) provides a
simulated reference implementation of a Trusted Execution Environment.

When enabled via `IdentityManager.use_tee()`, the system:
1.  Generates PQC keys inside the secure enclave boundary.
2.  **Seals** private keys using hardware-backed master keys before
    storing them on the host filesystem.
3.  Performs all **signing operations** (ML-DSA) inside the enclave,
    ensuring that raw private keys never enter the host's memory space.

---

## Tier 3 — Post-Quantum Resilience (Time)

### PQC Primitives

| Algorithm | NIST FIPS | Security Level | Key / Sig Sizes | Default use |
|---|---|---|---|---|
| ML-DSA-44 | FIPS 204 | Level 2 | pk=1 312 B, sk=2 528 B, sig=2 420 B | Low-risk tools |
| ML-DSA-65 | FIPS 204 | Level 3 | pk=1 952 B, sk=4 032 B, sig=3 293 B | Standard identity |
| ML-DSA-87 | FIPS 204 | Level 5 | pk=2 592 B, sk=4 896 B, sig=4 595 B | High-risk tools |
| ML-KEM-512 | FIPS 203 | Level 1 | pk=800 B, sk=1 632 B, ct=768 B | — |
| ML-KEM-768 | FIPS 203 | Level 3 | pk=1 184 B, sk=2 400 B, ct=1 088 B | Audit encryption |
| ML-KEM-1024 | FIPS 203 | Level 5 | pk=1 568 B, sk=3 168 B, ct=1 568 B | — |

Implementation: `dilithium-py` (ML-DSA) and `kyber-py` (ML-KEM) — Rust
FIPS-compliant reference implementations.

### PQC Agility (CASS)

CASS selects the ML-DSA variant based on tool risk at runtime. This agility
applies both to identity tokens and audit signatures. Three physically
separate key pairs are provisioned on disk:

| File | Variant | NIST Level |
|---|---|---|
| `~/.src/id_pqc_l2.key` | ML-DSA-44 | 2 |
| `~/.src/id_pqc_l3.key` | ML-DSA-65 | 3 |
| `~/.src/id_pqc_l5.key` | ML-DSA-87 | 5 |

Implementation: `PQCAgilityManager.get_required_level()` in
`src/security/pqc.rs`.

### Remote Attestation

On startup, the client generates a SHA-256 manifest of all critical security
source files and configuration files (defined by glob patterns in
`integrity.rs`), signed with an ML-DSA key. This manifest covers all Rust source files, configuration templates (`.toml`), and project metadata (e.g., `Cargo.toml`, `Makefile`).
 Remote servers verify this
manifest to confirm the agent is running an authentic, unmodified stack.
The system also detects unauthorized new files that match the critical
patterns, preventing backdoor installation.

Rebuild the manifest after any code update or configuration change:

```bash
llm-secure-cli-security manifest
```

### Audit Chain Continuity

- **Chained hashing** — each log entry includes a SHA-256 of the previous
  entry's hash, creating a tamper-evident chain.
- **Snapshot anchors** — on log rotation, a signed anchor entry records
  `snapshot_prev_hash` and `snapshot_first_hash` to maintain verifiability
  across file boundaries.
- **Merkle Tree anchoring** — a binary Merkle Root is computed over all entries
  on rotation and recorded in the security log.  This root can be submitted
  to an external immutable ledger (blockchain, transparency log) for public
  verification.
- **ML-KEM hybrid encryption** — audit logs are optionally encrypted with
  ML-KEM-768 + AES-256-GCM to guarantee future quantum confidentiality.
  Decrypt with:

  ```bash
  llm-secure-cli-security decrypt-log ~/.src/audit.jsonl -o decrypted.jsonl
  ```

**Audit log retention:** Back up `audit.jsonl` and its
`*.archive.*.jsonl` files.  Forward to a remote WORM store (SIEM) if
available.

---

## Security Performance & Latency Benchmarks

To maintain an agile user experience, `llm-secure-cli` optimizes for minimal operational overhead. The following measurements were captured on reference hardware (AMD Ryzen 5, WSL2).

### 1. Cryptographic Latency (PQC)
Measured using the Rust implementation on reference hardware.

| Tier | Component | Algorithm | Avg Latency (ms) |
|---|---|---|---|
| Tier 1 | Pattern-based Static Analysis | - | < 0.01 |
| Tier 2 | Identity Generation | ML-DSA-44 | 0.68 |
| Tier 2 | Identity Generation | ML-DSA-87 | 1.26 |
| Tier 3 | Audit Encryption | ML-KEM-768 | 0.09 |

### 2. Intent Verification Latency (Dual LLM)
Latency varies based on the provider and network conditions. We recommend "lite" models to minimize the "Security Speed Bump."

| Provider | Model | Accuracy | Avg Latency (ms) |
|---|---|---|---|
| Google | `gemini-3.1-flash-lite` | 100% | ~1297 |
| OpenAI | `gpt-5.4-nano` | 100% | ~1467 |
| Anthropic | `claude-haiku-4.5` | 100% | ~1295 |

---

## Security Configuration Reference

The primary security configuration is in `src/apps/defaults.toml`
(overridden by `~/.src/config.toml`):

```toml
[security]
# Security Level: "high" (Default) | "standard"
# Set to "standard" to downgrade PQC/integrity errors to warnings.
security_level           = "high"

# Workspace scope
allowed_paths            = ["."]
blocked_paths            = ["/etc", "/var", "/root", "~/.ssh"]

# Risk classification
high_risk_tools          = ["execute_command", "edit_file",
                            "create_or_overwrite_file", "read_url_content",
                            "brave_search"]
medium_risk_tools        = ["read_file_content", "grep_files"]
low_risk_tools           = ["list_files_in_directory", "search_files"]

# Dual LLM Verification
dual_llm_verification    = true
dual_llm_provider        = "google"
dual_llm_model           = "lite"

# PQC enforcement: "warn" | "strict_block"
pqc_enforcement          = "warn"

# Static analysis errors block execution
static_analysis_is_error = true
```

---

## Dependency Security

| Package | Purpose | Notes |
|---|---|---|
| `ring` / `openssl` | RSA, AES-256-GCM, key serialization | Rust-native; monitor upstream advisories |
| `pqcrypto` crates | ML-DSA / ML-KEM | FIPS-compliant implementations |
| `ciborium` | CBOR serialization for COSE tokens | Rust-native CBOR implementation |

> **Protocol Stability:** The COSE layer is implemented using Rust-native crates that are fully auditable and stable across library versions.

---

## Known Limitations & Security Trade-offs

### 1. Pattern-based Analysis vs. Obfuscation
The current Tier 1 static analysis utilizes a **pattern-based blocklist** (`src/security/static_analyzer.rs`). While highly efficient (<0.01ms), it is not a complete substitute for a full language-aware AST (Abstract Syntax Tree) parser. Sophisticated attackers may attempt to bypass this layer using:
- **Command Obfuscation**: e.g., `r\m -r\f /` or using hex-encoded strings.
- **Environment Manipulation**: e.g., using `export` or `alias` to hide destructive intent.

**Mitigation**: This layer is intended as a "Fast-Reject" gate. Real-world security relies on the **Defense-in-Depth** provided by Tier 2 (Human-in-the-Loop + Dual LLM) and Tier 3 (Audit Trail).

### 2. Probabilistic Intent Verification
Dual LLM Verification is a **probabilistic** defense. While it achieved 100% accuracy in our initial 10-case test suite, LLMs can hallucinate or fail to catch "jailbreak" style prompt injections. The confidence threshold setting is critical for balancing security and usability.

### 3. ML-DSA COSE algorithm identifier (`alg=−48`)
...

---

## Production Hardening Roadmap

While the current implementation provides a robust software-defined security
stack, the following steps are recommended for production environments
requiring high-assurance guarantees:

1. **Hardware Sovereignty (TEE):**
   The software logic (especially PQC key management) is now abstracted via 
   `TEEPQCBackend`. This reference implementation demonstrates how to 
   protect private keys using enclave-based sealing and isolated signing 
   memory (e.g., Intel SGX, AWS Nitro).

2. **Formal Cryptographic Audit:**
   The PQC implementations used in this project should undergo a 
   professional third-party cryptographic audit before being used 
   to protect high-value assets.

3. **Policy & Retention Configuration:**
   Adjust `max_audit_archives` and `max_audit_log_lines` in the deployment
   `config.toml` to satisfy specific legal (GDPR/CCPA) or regulatory data
   retention requirements.

4. **Hardware Security Modules (HSM) & KMS:**
   Enterprise identity management is now supported via the `TrustResolver` 
   interface. This allows integrating with a centralized KMS (Key Management 
   Service) or HSM for a unified root of trust.

