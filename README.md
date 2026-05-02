# llm-secure-cli: Unified OpenAI-Compatible CLI for AI Agents

`llm-secure-cli` (binary name: `llsc`) is a high-assurance command-line tool designed for interacting with Large Language Models (LLMs). It provides a unified, stable interface for any OpenAI-compatible API, including **OpenRouter, OpenAI, Ollama, and LiteLLM**, prioritizing cognitive focus, secure execution, and extensible automation.

[English] | [日本語](#japanese-description)

---

###  Purpose & Positioning

Enterprise adoption of autonomous AI agents faces a fundamental, unsolved challenge: **how do you grant an AI meaningful agency while maintaining the security and governance standards that organizations require?** This project is one engineer's attempt to answer that question in working code.

`llm-secure-cli` was built primarily as a **personal daily-use tool** and as a **portfolio artifact** — a concrete demonstration of how CISSP/CISA/CCSP-level security principles (Zero Trust, ABAC, non-repudiation, PQC resilience) can be applied to the novel threat surface introduced by autonomous LLM agents.

**This tool is not certified or validated for enterprise production use.** No formal third-party security audit has been conducted, and the PQC primitives rely on Rust implementations that have not undergone independent cryptographic review. Deploying this in a regulated or mission-critical environment without additional validation would be inappropriate.

Instead, its recommended uses are:

-  **As a reference architecture** — for security engineers and architects exploring what a high-assurance agentic system *could* look like.
-  **As an evaluation platform** — for studying the practical trade-offs between AI agent autonomy and hybrid high-assurance security controls.
-  **As a design provocation** — a starting point for organizational discussions on agentic AI governance, not a finished answer.

The accompanying [Technical Report](paper/comprehensive_framework/paper.pdf) details the threat model and architectural decisions behind this framework.

---

<p align="center">
  <img src="images/architecture.png" width="800" alt="llm-secure-cli Simplified Architecture Flow" />
  <br>
  <em>Simplified Architecture Flow</em>
</p>

---

##  Quick Start

1.  **Install**:
    ```bash
    # Install from source
    git clone https://github.com/yosh95/llm-secure-cli-rust.git
    cd llm-secure-cli-rust
    cargo install --path .
    # Generate a manifest file
    llsc identity manifest
    ```
2.  **Set API Keys**: `llsc` uses OpenAI-compatible APIs. Set keys for your chosen provider.
    ```bash
    # Example for OpenRouter
    export OPENROUTER_API_KEY="your-api-key"
    
    # Generic provider name support
    # ANYNAME_API_KEY can be used if you define [ANYNAME] in config.toml
    ```
3.  **Chat**: Type `llsc` to start an interactive session.
    *   **Automatic Initialization**: On the first run, `~/.llm_secure_cli/config.toml` is automatically created.
    *   **Brave Search**: Built-in support for the Brave Search API is available for comprehensive searching across all providers (requires `BRAVE_API_KEY`).
4.  **Configure (Optional)**: Ollama is the default provider. To use OpenRouter or others, edit the configuration file:
    ```bash
    # Edit ~/.llm_secure_cli/config.toml
    ```
5.  **Help**: Type `/help` inside the chat to see all commands.

### Docker Isolation (Optional)
Run the agent in a completely isolated container to protect your host system. In `high` security mode (default), you must initialize the integrity manifest within the mounted volume.

1. **Build**: `docker build -t llm-secure-cli .`
2. **Setup API Keys**:
   - **Option A: `.env` file (Recommended)**: Place a `.env` file in your host's `~/.llm_secure_cli/` directory.
     ```bash
     # ~/.llm_secure_cli/.env
     OPENROUTER_API_KEY=sk-...
     OPENAI_API_KEY=sk-...
     ```
   - **Option B: Environment Variables**: Pass them via the `-e` flag during `docker run`.
3. **Initialize (Required for first run or after updates)**:
   Generate keys and authorize the container binary.
   ```bash
   # Generate identity keys
   docker run -it --rm -v ~/.llm_secure_cli:/root/.llm_secure_cli llm-secure-cli identity keygen
   # Create integrity manifest
   docker run -it --rm -v ~/.llm_secure_cli:/root/.llm_secure_cli llm-secure-cli identity manifest
   ```
4. **Run**:
   ```bash
   docker run -it --rm \
     -v ~/.llm_secure_cli:/root/.llm_secure_cli \
     -v $(pwd):/workspace \
     llm-secure-cli "Summarize the files in this directory"
   ```

> **Note**: If you rebuild or pull a new image, the binary hash will change. You must run the `identity manifest` command again to authorize the new version.

### One-Shot Examples
```bash
# Ask a question using the default provider (Ollama)
llsc "What is the capital of France?"

# Use a specific provider and model (e.g., OpenRouter)
llsc -p openrouter -m default "Explain quantum computing"

# Output raw text to a file (disables Markdown rendering)
llsc --stdout --raw "Write a python script to sort files" > sort.py
```

## Core Features

- **Unified Provider Access**: Seamlessly switch between any OpenAI-compatible APIs, such as **OpenRouter, OpenAI, Ollama, and LiteLLM**.
- **Autonomous Agent**: Let the AI manage files and search the web using **Brave Search**.
- **High-Assurance via Dual LLM**: Every high-risk tool call is verified by a secondary LLM (via an OpenAI-compatible endpoint) to ensure intent alignment, balancing flexibility and security.
- **Config-free Execution**: Start using immediately by just providing an environment variable.
- **MCP (Model Context Protocol) Support**: Connect to remote resources or services via custom servers configured in `config.toml`.
- **Multimodal capabilities**: Support for Images, PDFs, Audio, and Video (as supported by the underlying OpenAI-compatible model).
- **Operational Stability**: A clean, flicker-free UI designed for long-term "Deep Work" sessions and SSH-based environments.
- **Human-in-the-Loop**: All critical actions (file edits, code execution) require explicit human approval by default (configurable via `auto_approval_level`).

### Autonomous Agent & Tool Use
The AI agent autonomously uses tools to perform complex tasks, such as file management and web search. Web search is powered by the **Brave Search API** for comprehensive results. To maintain audit integrity and PQC signatures, all external data retrieval is cryptographically signed and logged.

---

## Security & Governance (High-Assurance Framework)

As a tool designed with **CISSP/CISA/CCSP** principles and **EU AI Act** compliance in mind, `llm-secure-cli` implements a multi-layered security architecture to mitigate the risks associated with autonomous AI agents.

### 1. Access Control (AI-native ABAC & Semantic Guardrails)
`llm-secure-cli` implements a modern **Attribute-Based Access Control (ABAC)**, moving away from fragile, platform-dependent static rules.
- **AI-native Policy Engine (Dual LLM)**: Replaces complex regex blocklists with a hardcoded **Security Constitution**. The system automatically gathers context (OS, User, Directory, Git status) and uses a secondary LLM to judge risks semantically using structured verdicts (ALLOW/BLOCK). This avoids the quagmire of platform-dependent static rules.
- **Path Guardrails (Physical Boundary)**: Paths are recursively normalized and validated against a whitelist. Even for new files, the system resolves the physical parent directory to prevent symlink-based escapes.
- **Risk-based Scaling (CASS)**: Security requirements (PQC signature level, audit encryption) automatically scale based on the tool's risk level (CRITICAL/HIGH/MEDIUM/LOW) via the **CASS (Context-Adaptive Security Scaling)** orchestrator.
- **Intent Verification**: Every high-risk action is cross-verified by a separate, lightweight "Verifier" LLM to ensure the proposed tool call aligns with the user's original intent.
- **Minimalist Fast-fail**: A lightweight syntactic check still blocks obviously malicious characters and shell invocation patterns in **nanoseconds**, while the heavy lifting of security judgment is shifted to the Dual LLM.
- **Verifier Fallback Policy**: When the dual LLM verifier is unavailable (network error, API failure), the behavior is controlled by `verifier_fallback`: `require_approval` (default, forces human approval) or `block` (blocks all tool calls).
- **Auto-Approval Levels**: The `auto_approval_level` setting controls which tool calls can proceed without human intervention: `none` (default, all require approval), `low` (auto-approve low-risk), `medium` (auto-approve low and medium-risk).
- **Physical Isolation (Docker)**: The agent can be run inside a Docker container to provide a hard boundary between the AI and the host system.

### 2. Identity & Non-Repudiation (Experimental Reference)
- **Distributed Trust Model**: Implements a decentralized identity model where clients and servers only exchange public keys. This is designed to explore how to prevent lateral movement if a single component is compromised; however, it requires thorough evaluation before use in production environments.
- **Hybrid Identity Tokens**: Uses **COSE (RFC 9052)** binary structures combining **Ed25519** with **Post-Quantum Cryptography (ML-DSA)**. This serves as a reference for how long-term non-repudiation might be handled in autonomous agent systems.
- **Client Integrity Attestation**: The client generates a signed manifest of its own source code state to demonstrate the integrity of the execution environment.
- **Bi-directional Verification**: Tool results can be signed by the responder, allowing the requester to verify that the observations are authentic and untampered within the protocol's scope.

### 3. Observability & Audit Compliance (Tier 3 Reference Implementation)
- **Tamper-Evident Audit Logs**: Audit trails are protected using **Chained Hashing** and optionally encrypted with **ML-KEM (Kyber)** for confidentiality.
- **Merkle Tree Anchoring**: The Tier 3 implementation uses Merkle Trees to anchor log batches, demonstrating an architecture to prevent historical revisionism and provide compact proofs of session integrity.

---

##  Advanced Commands & Power User Tips

### Command-Line Flags
```bash
llsc [SOURCES...]                    # Start interactive chat (optional initial text/files)
llsc -p <provider>                   # Start with specific provider
llsc -m <model>                      # Start with specific model alias
llsc --stdout                        # Non-interactive mode, output to stdout
llsc --raw                           # Disable Markdown rendering (use with --stdout)
llsc --mcp-server                    # Run as an MCP server (stdio transport)
llsc --session <path>                # Load a saved session on startup
llsc --debug                         # Enable debug logging
llsc "query"                         # One-shot query
```

### Subcommands
```bash
llsc models [provider]               # List available models (optionally for a specific provider)
llsc models models <provider> -v     # Verbose model listing
llsc identity keygen                 # Generate Ed25519 and PQC (ML-DSA/ML-KEM) key pairs
llsc identity manifest               # Rebuild integrity manifest for system verification
llsc identity verify                 # Run full integrity verification
llsc identity verify-session <id>    # Verify session integrity using Merkle Anchor
llsc identity list-sessions          # List available anchored sessions
llsc decrypt-log <input> [-o <out>]  # Decrypt PQC-encrypted audit logs
```

### Interactive Session Commands
Inside the `llsc` interactive session:
- `/help`, `/h`: Show help message.
- `/quit`, `/q`: Exit the application.
- `/system [on|off]`: Show or toggle system prompt status.
- `/edit`, `/e`: Edit current buffer in external editor.
- `/clear`, `/c`: Clear conversation history.
- `/info`, `/i`: Show session info, integrity, and security status.
- `/debug`: Toggle live debug mode.
- `/raw`: Show conversation as raw text.
- `/dump`: Dump conversation history as JSON.
- `/save <path>` / `/load <path>`: Manage conversation history.
- `/attach <path/URL>`: Add a file or website content to the context.
- `/tools [on|off]`: Show or toggle autonomous tool use status.
- `/model`, `/m [<alias>]`: Switch or list models for current provider.
- `/provider`, `/p [<name>]`: Switch or list providers.
- `/checkpoint`, `/cp`: Checkpoint (Summarize and clear history).

### Keybindings
- **Newline**: `Ctrl+J` (Insert a newline without submitting).
- **Clear Screen**: `Ctrl+L`.
- **History**: `Up/Down` arrows to navigate.
- **Interrupt**: `Ctrl+C` to cancel the current thinking process or exit the session.

###  Logging & Troubleshooting
By default, `llsc` and its related tools suppress all informational logs and only show `WARNING` or `ERROR` messages. If you encounter issues:
- **Enable Debug Mode**: Add the `--debug` flag when launching, or use `/debug` in an interactive session:
  ```bash
  llsc --debug "query"
  llsc --debug identity verify
  ```
- **MCP Debugging**: To troubleshoot the MCP server (which is often spawned by a third-party client), use the `MCP_DEBUG` environment variable:
  ```bash
  export MCP_DEBUG=1
  # Then start your MCP client (e.g., Claude Desktop)
  ```
- **Log Location**: Security and audit logs are stored in `~/.llm_secure_cli/logs/audit.jsonl`.

### Power User Tips
- **Backgrounding (`Ctrl+Z`)**: Suspend the session to perform shell operations, then use `fg` to return.
- **Prompt Continuation**: Use `\` at the end of a line or open a code block with ``` to enter multi-line mode automatically.
- **External Editor**: Use `/edit` (or `/e`) for composing complex prompts in your default editor (`vim`, `nano`, etc.).
- **Model-specific Tool Disabling**: For models that do not support tool use (e.g., image generation models), you can pre-configure them to disable tools automatically in `config.toml`:
  ```toml
  [openrouter.models]
  nano-banana-2 = { model = "google/gemini-3.1-flash-image-preview", image_generation = true, tools = false }
  ```
- **Disabling Tools Manually**: Use `/tools off` to prevent errors when using a model that doesn't support function calling.

## Security Configuration Reference

The primary security configuration is in `src/config/defaults.toml` (overridden by `~/.llm_secure_cli/config.toml`):

```toml
[general]
unified_default_provider = "ollama"
pdf_as_base64 = true
request_timeout = 1800
command_timeout = 300
max_audit_log_lines = 10000
max_audit_archives = 10
max_chat_log_lines = 5000
max_chat_archives = 5

[security]
# Security Level: "high" (Default) | "standard"
security_level = "high"

# Workspace scope
allowed_paths = ["."]

# Auto-Approval Policy: "none" (default) | "low" | "medium"
auto_approval_level = "none"

# Tool whitelist
allowed_tools = [
    "list_files_in_directory", "read_file_content", "grep_files",
    "search_files", "edit_file", "create_or_overwrite_file",
    "read_url_content", "brave_search", "execute_command"
]

# Risk classification
high_risk_tools = ["execute_command", "edit_file",
                   "create_or_overwrite_file", "read_url_content",
                   "brave_search"]
medium_risk_tools = ["read_file_content", "grep_files"]
low_risk_tools = ["list_files_in_directory", "search_files"]

# Dual LLM Verification
dual_llm_verification = true
dual_llm_provider = "ollama"
dual_llm_model = "default"
dual_llm_confidence_threshold = 0.7

# Verifier Fallback Policy: "require_approval" (default) | "block"
verifier_fallback = "require_approval"

# Static analysis errors block execution
static_analysis_is_error = true
```

### MCP Server Configuration
Configure remote MCP servers in `config.toml`:

```toml
[[mcp_servers]]
name   = "my-server"
command = "ssh"
args   = ["user@host", "llsc", "--mcp-server"]
zero_trust = true
```

## Development & Benchmarks
To run the local security primitive benchmarks (Static Analysis, PQC Keygen/Sign/Verify):
```bash
cargo bench --bench benchmark_local
```

To run the internal Dual LLM verification scenarios (requires API keys):
```bash
cargo bench --bench benchmark_dual_llm
# Or with a custom scenarios JSON file:
cargo bench --bench benchmark_dual_llm -- path/to/your_scenarios.json
```

##  License
Licensed under [Apache License 2.0](LICENSE). 

For detailed architectural insights and the academic background of our security framework, please refer to the **[Technical Report (Pre-print)](paper/comprehensive_framework/paper.pdf)**.

---

<a id="japanese-description"></a>

# llm-secure-cli: OpenAI互換API対応 統合AIエージェントCLI

`llm-secure-cli`（バイナリ名：`llsc`）は、**OpenRouter, OpenAI, Ollama, LiteLLM** をはじめとする任意のOpenAI互換APIを一元的に操作できる、高い安全性を備えたCLIツールです。開発者の「深い集中（Deep Work）」を妨げない安定した対話環境と、プロフェッショナルな要求に応える高度なセキュリティ機能を両立しています。

[English] | [日本語]

---

###  位置づけと目的

企業における自律型 AI エージェントの活用には、根本的かつ未解決の課題があります。**「AIに十分な自律性を与えながら、組織が求めるセキュリティ標準とガバナンスをどう両立させるか」**――本プロジェクトは、その問いに対してエンジニアが動くコードで答えを試みたものです。

`llm-secure-cli` は主に**個人の日常利用ツール**として、また**ポートフォリオ**として開発されました。Zero Trust・ABAC・非否認性・耐量子暗号といった CISSP/CISA/CCSP レベルのセキュリティ原則を、自律型 LLM エージェントがもたらす新しい脅威対象に適用すると、実装としてどのような形になるかを具体的に示すことを目的としています。

**本ツールは、エンタープライズ本番環境への適用を保証・認定するものではありません。** 第三者による正式なセキュリティ監査は実施されておらず、PQC プリミティブは独立した暗号学的レビューを受けていない Rust 実装に依存しています。規制対象業務やミッションクリティカルな環境への追加検証なしでの展開は推奨しません。

本プロジェクトの推奨される活用方法は以下のとおりです。

-  **参照アーキテクチャとして** ― 高保証なエージェントシステムが「どのような設計になりえるか」を探求するセキュリティエンジニア・アーキテクト向け。
-  **評価プラットフォームとして** ― AI エージェントの自律性とハイブリッドな高保証セキュリティ制御の間にある実践的なトレードオフを検討するための実験基盤として。
-  **設計上の問いかけとして** ― 組織内における AI エージェントのガバナンス議論の起点として。完成した答えではなく、問いを深めるための素材として。

設計の背景にある脅威モデルとアーキテクチャ上の意思決定については、[テクニカルレポート](paper/comprehensive_framework/paper.pdf)で詳述しています。

---

<p align="center">
  <img src="images/architecture.png" width="800" alt="llm-secure-cli 簡易アーキテクチャフロー" />
  <br>
  <em>簡易アーキテクチャフロー (TikZ版)</em>
</p>

---

##  クイックスタート

1.  **インストール**:
    ```bash
    # ソースからインストール
    git clone https://github.com/yosh95/llm-secure-cli-rust.git
    cd llm-secure-cli-rust
    cargo install --path .
    # manifestの生成
    llsc identity manifest
    ```
2.  **APIキーの設定**: OpenAI互換APIを使用します。利用するプロバイダーのキーを設定してください。
    ```bash
    # OpenRouterの例
    export OPENROUTER_API_KEY="your-api-key"
    
    # ANYNAME_API_KEY という形式で環境変数を設定すれば、
    # config.toml で定義した [ANYNAME] セクションと自動で紐付けられます。
    ```
3.  **対話開始**: `llsc` コマンドでスタート。
    *   **設定の自動生成**: 初回起動時に `~/.llm_secure_cli/config.toml` が自動的に作成されます。
    *   **Brave Search**: すべてのプロバイダーで利用可能な共通のWeb検索ツールとして Brave Search API をサポートしています（`BRAVE_API_KEY` が必要）。
4.  **詳細設定 (任意)**: デフォルトでは Ollama を使用します。OpenRouter やその他のプロバイダーを使用する場合は設定ファイルを編集します。
    ```bash
    # ~/.llm_secure_cli/config.toml を編集して設定を調整してください。
    ```
5.  **ヘルプ**: チャット内で `/help` と入力するとコマンド一覧が表示されます。

### Docker による隔離環境 (任意)
ホストシステムを保護するために、完全に隔離されたコンテナ内でエージェントを実行できます。`high`セキュリティモード（デフォルト）では、ボリュームマウント時に整合性マニフェストを初期化する必要があります。

1. **ビルド**: `docker build -t llm-secure-cli .`
2. **APIキーの設定**:
   - **オプションA: `.env` ファイルを使用 (推奨)**: ホストの `~/.llm_secure_cli/.env` にキーを記述します。
     ```bash
     # ~/.llm_secure_cli/.env
     OPENROUTER_API_KEY=sk-...
     OPENAI_API_KEY=sk-...
     ```
   - **オプションB: 環境変数**: `docker run` 時に `-e` オプションで渡します。
3. **初期化 (初回および更新時に必須)**:
   鍵を生成し、コンテナ内のバイナリを承認します。
   ```bash
   # 署名用キーの生成
   docker run -it --rm -v ~/.llm_secure_cli:/root/.llm_secure_cli llm-secure-cli identity keygen
   # 整合性マニフェストの作成
   docker run -it --rm -v ~/.llm_secure_cli:/root/.llm_secure_cli llm-secure-cli identity manifest
   ```
4. **実行**:
   ```bash
   docker run -it --rm \
     -v ~/.llm_secure_cli:/root/.llm_secure_cli \
     -v $(pwd):/workspace \
     llm-secure-cli "このディレクトリのファイルを要約して"
   ```

> **注意**: Dockerイメージをビルドし直したり更新したりすると、バイナリのハッシュ値が変わります。その場合は再度 `identity manifest` を実行して新しいバージョンを承認してください。

### ワンショット実行例
```bash
# デフォルトのプロバイダー（Ollama）で質問する
llsc "フランスの首都はどこですか？"

# 特定のプロバイダーとモデルを指定する（例：OpenRouter）
llsc -p openrouter -m default "量子コンピュータについて説明して"

# 生のテキストをファイルに出力する（Markdownレンダリングを無効化）
llsc --stdout --raw "ファイルをソートするPythonスクリプトを書いて" > sort.py
```

## 主な機能 (実用ツールとして)

- **統合インターフェース**: `llsc` コマンド一つで **OpenRouter, OpenAI, Ollama, LiteLLM** などのあらゆるOpenAI互換APIにアクセス。
- **自律型エージェント**: ファイル操作、Web検索、URL解析をAIが自律的に実行。Web検索は **Brave Search** を使用します。
- **Dual LLM による高保証**: 全ての高リスクなツール実行は、OpenAI互換エンドポイントを介したセカンダリLLMによって検証され、柔軟性と安全性を両立しています。
- **設定不要の即時利用**: 環境変数を設定するだけで、セットアップの手間なく利用可能。
- **MCP (Model Context Protocol) 対応**: `config.toml` に設定されたリモートサーバーや外部サービスとの連携をサポート。
- **マルチモーダル対応**: 画像、PDF、音声、動画の入力をサポート（基盤となるモデルの対応状況に依存）。
- **集中力を削がないUI**: 画面のちらつきを抑え、SSH越しでも安定して動作するクリーンなターミナル出力。
- **Human-in-the-Loop**: ファイル編集やコード実行などの重要な操作は、デフォルトで人間の明示的な承認を必要とします（`auto_approval_level` で設定可能）。

### 自律型エージェントのツール実行
AIがファイル操作、Web検索などのツールを自律的に使用し、複雑なタスクを遂行します。Web検索は **Brave Search API** を利用します。監査の健全性を維持しPQC署名を確保するため、すべての外部データ取得が暗号学的に署名・記録されます。

## セキュリティとガバナンス (プロフェッショナル向け)

本ツールは **CISSP/CISA/CCSP** の各ドメインにおける管理策、および **EU AI Act（欧州AI法）** の技術的要件を意識して設計されています。

### 1. 属性ベースアクセス制御 (AI-native ABAC)
`llm-secure-cli` は、OSに依存する脆弱な静的ルールを廃止し、最新の **属性ベースアクセス制御 (ABAC)** を採用しています。
- **AIネイティブ・ポリシーエンジン**: 複雑なTOMLルールの代わりに、ハードコードされた **セキュリティ憲法（Security Constitution）** を使用します。システムは自動的に実行コンテキスト（OS、ユーザー、ディレクトリ、Gitステータス）を収集し、セカンダリLLMがALLOW/BLOCKの構造化判定を下します。
- **リスクベース・スケーリング (CASS)**: ツールのリスクレベル（CRITICAL/HIGH/MEDIUM/LOW）に応じて、要求されるセキュリティ強度が自動的に変化します。
- **意図の検証 (Dual LLM)**: 全ての高リスクな操作は、軽量な「検証用LLM」によって元のプロンプトおよびセキュリティ憲法と照合されます。
- **検証フォールバックポリシー**: Dual LLM 検証が利用できない場合（ネットワークエラー等）、`verifier_fallback` 設定に従います。デフォルトは `require_approval`（人間の承認が必要）、`block`（全ツール呼び出しを遮断）も選択可能です。
- **自動承認レベル**: `auto_approval_level` により、ツール実行の自動承認を制御します。`none`（デフォルト、全て承認必要）、`low`（低リスク自動承認）、`medium`（低・中リスク自動承認）。
- **物理的隔離 (Docker)**: エージェントをDockerコンテナ内で実行することで、AIとホストシステムの間に強力な境界を設けることができます。
- **最小限の高速チェック**: シェル起動パターン（`sh -c`等）や制御文字など明らかに不正な入力は軽量な静的チェックで即座に遮断しますが、高度なリスク判断はDual LLMにシフトします。

### 2. アイデンティティと非否認性 (実験的参照実装)
- **分散型トラストモデル**: クライアントとサーバーが公開鍵のみを交換する分散型アイデンティティモデルを実装。特定のコンポーネントが侵害された際の横展開を防止する手法を探求していますが、エンタープライズ領域での利用には十分な評価が必要です。
- **ハイブリッド署名**: **COSE (RFC 9052)** を採用し、**Ed25519** と **耐量子暗号 (ML-DSA)** を組み合わせた署名を実装。将来的な非否認性の確保に向けた参照実装としての位置づけです。
- **完全性検証**: クライアント自身のソースコードの状態を署名付きマニフェストで証明し、実行環境の健全性を担保します。
- **双方向検証**: ツールの実行結果に署名を付与し、受信側がデータの正当性をプロトコルの範囲内で検証可能です。

### 3. 観測可能性と監査ログ (Tier 3 参照実装)
- **改ざん防止監査ログ**: ハッシュ連鎖（Chained Hashing）によるログ保護と、**ML-KEM (Kyber)** による機密性保護を実装しています。
- **Merkle Tree アンカリング**: Tier 3 実装として Merkle Tree によるログバッチの固定を導入。履歴の改ざんを防止し、セッションの整合性を証明するアーキテクチャのプロトタイプです。

---

###  高度なコマンドとパワーユーザー向け機能

### コマンドラインフラグ
```bash
llsc [SOURCES...]                    # インタラクティブチャットを開始（初期テキスト/ファイル指定可）
llsc -p <provider>                   # プロバイダーを指定して開始
llsc -m <model>                      # モデルエイリアスを指定して開始
llsc --stdout                        # 非対話モード、標準出力へ出力
llsc --raw                           # Markdownレンダリングを無効化（--stdoutと併用）
llsc --mcp-server                    # MCPサーバーとして実行（stdioトランスポート）
llsc --session <path>                # 保存済みセッションを読み込んで起動
llsc --debug                         # デバッグログを有効化
llsc "query"                         # ワンショットクエリ
```

### サブコマンド
```bash
llsc models [provider]               # 利用可能なモデルを表示（プロバイダー指定可）
llsc models <provider> <models> -v    # 詳細なモデル一覧
llsc identity keygen                  # Ed25519 および PQC (ML-DSA/ML-KEM) 鍵ペアの生成
llsc identity manifest                # システム検証のための整合性マニフェスト再構築
llsc identity verify                  # 完全な整合性検証の実行
llsc identity verify-session <id>     # Merkle Anchorを用いたセッション整合性の検証
llsc identity list-sessions           # アンカーされた利用可能なセッションの一覧表示
llsc decrypt-log <input> [-o <out>]   # PQC暗号化監査ログの復号
```

### インタラクティブセッションコマンド
`llsc` インタラクティブセッション内で利用可能なコマンド:
- `/help`, `/h`: ヘルプメッセージを表示。
- `/quit`, `/q`: アプリケーションを終了。
- `/system [on|off]`: システムプロンプトのステータス表示・切り替え。
- `/edit`, `/e`: 外部エディタで現在の入力を編集。
- `/clear`, `/c`: 会話履歴をクリア。
- `/info`, `/i`: セッション情報、整合性、およびセキュリティステータスを表示。
- `/debug`: ライブデバッグモードの切り替え。
- `/raw`: 会話をそのままのテキストとして表示。
- `/dump`: 会話履歴をJSON形式でダンプ。
- `/save <path>` / `/load <path>`: 会話履歴の保存・読み込み。
- `/attach <path/URL>`: ファイルやウェブサイトのコンテンツをコンテキストに追加。
- `/tools [on|off]`: ツールの自律実行ステータスの表示・切り替え。
- `/model`, `/m [<alias>]`: モデルの切り替え・一覧表示。
- `/provider`, `/p [<name>]`: プロバイダーの切り替え・一覧表示。
- `/checkpoint`, `/cp`: チェックポイント (会話の要約と履歴のクリア)。

### キーバインド
- **改行**: `Ctrl+J` (送信せずに次の行へ移動)。
- **画面クリア**: `Ctrl+L`。
- **履歴移動**: `↑`/`↓` キー。
- **中断**: `Ctrl+C` (生成の中断、またはセッションの終了)。

###  ログとトラブルシューティング
デフォルトでは、`llsc` およびその関連ツールは、`WARNING` または `ERROR` メッセージのみを表示し、情報のログは表示しません。
- **デバッグモードの有効化**: ツールの起動時に `--debug` フラグを追加するか、インタラクティブセッションで `/debug` コマンドを使用します。
  ```bash
  llsc --debug "query"
  llsc --debug identity verify
  ```
- **MCP のデバッグ**: 3rdパーティのクライアント（Claude Desktop等）から起動される MCP サーバーのトラブルシューティングには、`MCP_DEBUG` 環境変数を使用します。
  ```bash
  export MCP_DEBUG=1
  # その後、お使いの MCP クライアントを起動してください。
  ```
- **ログの場所**: セキュリティおよび監査ログは `~/.llm_secure_cli/logs/audit.jsonl` に保存されています。

### パワーユーザー向け機能
- **一時中断 (`Ctrl+Z`)**: セッションをバックグラウンドに送り、シェルに戻る。`fg` で復帰可能。
- **入力の継続**: 行末に `\` を入力するか、` ``` ` でコードブロックを開始することで、自動的に複数行入力モードになります。
- **外部エディタ**: `/edit` (または `/e`) と入力することで、`vim` や `nano` などのデフォルトエディタで編集できます。複雑なプロンプトを作成する際に便利です。
- **モデルごとのツール自動無効化**: 画像生成モデルなど、ツール利用に対応していないモデルに対して、`config.toml` で自動的にツール機能をオフに設定できます。
  ```toml
  [openrouter.models]
  nano-banana-2 = { model = "google/gemini-3.1-flash-image-preview", image_generation = true, tools = false }
  ```
- **ツール機能の手動無効化**: `/tools off` コマンドでツール送信を一時的に無効化できます。

## セキュリティ設定リファレンス

プライマリのセキュリティ設定は `src/config/defaults.toml` にあり、`~/.llm_secure_cli/config.toml` で上書きできます：

```toml
[general]
unified_default_provider = "ollama"
pdf_as_base64 = true
request_timeout = 1800
command_timeout = 300
max_audit_log_lines = 10000
max_audit_archives = 10
max_chat_log_lines = 5000
max_chat_archives = 5

[security]
# セキュリティレベル: "high" (デフォルト) | "standard"
security_level = "high"

# ワークスペーススコープ
allowed_paths = ["."]

# 自動承認ポリシー: "none" (デフォルト) | "low" | "medium"
auto_approval_level = "none"

# ツールホワイトリスト
allowed_tools = [
    "list_files_in_directory", "read_file_content", "grep_files",
    "search_files", "edit_file", "create_or_overwrite_file",
    "read_url_content", "brave_search", "execute_command"
]

# リスク分類
high_risk_tools = ["execute_command", "edit_file",
                   "create_or_overwrite_file", "read_url_content",
                   "brave_search"]
medium_risk_tools = ["read_file_content", "grep_files"]
low_risk_tools = ["list_files_in_directory", "search_files"]

# Dual LLM 検証
dual_llm_verification = true
dual_llm_provider = "ollama"
dual_llm_model = "default"
dual_llm_confidence_threshold = 0.7

# 検証フォールバックポリシー: "require_approval" (デフォルト) | "block"
verifier_fallback = "require_approval"

# 静的解析エラーは実行をブロック
static_analysis_is_error = true
```

### MCPサーバー設定
`config.toml` でリモートMCPサーバーを設定します：

```toml
[[mcp_servers]]
name   = "my-server"
command = "ssh"
args   = ["user@host", "llsc", "--mcp-server"]
zero_trust = true
```

## 開発とベンチマーク
ローカルのセキュリティプリミティブ（静的解析、PQC鍵生成/署名/検証）のベンチマークを実行するには：
```bash
cargo bench --bench benchmark_local
```

内部的な Dual LLM 検証シナリオを実行するには（APIキーが必要）：
```bash
cargo bench --bench benchmark_dual_llm
# またはカスタムのシナリオJSONファイルを指定する場合：
cargo bench --bench benchmark_dual_llm -- path/to/your_scenarios.json
```

##  License
Licensed under [Apache License 2.0](LICENSE). 

設計の背景にある脅威モデルとアーキテクチャ上の意思決定については、**[テクニカルレポート（プレプリント）](paper/comprehensive_framework/paper.pdf)** を参照してください。