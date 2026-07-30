# vNext shadow prototype

`FRAMEWORK-REVIEW.md` 14章のThin Kernel仮説を、現行CLIへ接続せず検証するための実験実装です。公開APIでも移行先の確定実装でもありません。

## 検証できること

DB更新とSQS送信を含むfixtureに対して、次の順序を決定的に再現します。

```text
Project Snapshot
  → Rule compile
  → source observation and binding
  → typed fact detection
  → Thin Kernel
  → NextAction
  → Generated Context
  → Result submit
  → Thin Kernel再評価
```

- 複数Ruleが選ぶ同じRequirement Instanceの重複排除
- Signal候補を人またはAnalystが確認してからのRule適用
- Detector coverage未報告・未完了時の`blocked-detection`
- RustのPython構文解析によるDB書込み・message publishの観測
- Git管理された解析rootの宣言漏れ、parse失敗、binding未解決、未対応観測のfail-closed化
- 未知のrepository fact kindの拒否
- 組込みSignal Catalogによる未知Signal・不正binding参照のRule compile拒否
- Contract gapのHuman Authorityへの差し戻し
- Human回答をDecisionとContractへ反映するまで停止すること
- 実装前Challenge、Evidence、実装後Challenge
- 参照元のdigest変更によるResultのstale判定
- Requirement InstanceごとのContext source manifest
- 条項固有`applies_to`の継承解決と、対象に一致するContract条項本文・digestのContext投影
- 同じAction内でのoutcome単位のfreshness判定
- `applies_to`に一致しないContract変更からのResult保護
- detector・signal・bindingから成る安定したfingerprintによるSignal確認結果の再利用
- 根拠ref・digestを分離したevidence versionと、`not-applicable`の根拠変更時の再確認
- `result.build`へ固定した実装前Resultを基準に、通常のbuild後はEvidence工程から再開
- 新規・変更candidateだけを提示する差分risk review
- candidateからNext Actionまでを追跡する機械可読・テキスト`explain`
- Framework protocol、Detector、Rule source・Indexを固定するFramework lock
- Releaseの取得元ID、配布物digest、署名鍵IDを固定するFramework lock v2
- Ed25519署名済みoffline bundleの検証、原子的なinstall、lock切替・rollback
- Git管理された取得元からのHTTPS downloadと安全なtar展開
- Git管理Recordを模したFilesystem Project Storeとプロセス再起動後のState再現
- Shared Contract・Decisionの新規既定rootを`contracts/`・`decisions/`へ分離
- Resultの排他的追記とContract・Decisionの原子的更新
- 読取り時digestと排他lockによるShared Contractのstale更新拒否
- 削除・破損から再生成できる`.agentic/cache/`へのwrite-through
- cleanな実Git cloneからの`ready-to-merge`再現
- Git revision、tracked artifact、未commit変更を検査するCI Evaluator
- 人向け本文を保持するChange・Contract・DecisionのMarkdown Record
- typed blockだけを更新し、YAML版と同じSnapshotを生成するMarkdown codec
- Change・Contract・Decision・Result・Evidenceの言語非依存JSON Schema検証
- 6種類のResult payload Schemaと、Result種別ごとの許可Role検証
- outcomeの結論・根拠参照と、発行Contextに対する参照整合性の検証
- 全ChangeのEvidence履歴と現在の入力digestから生成する条項単位のContract Health表示
- Schema bundleのversionとdigestを固定するFramework lock
- JSONだけで構成したcanonicalization・Schema・Rule Compiler・Detector・Thin Kernel・Context Compiler・Project Snapshot・Framework lock・Result submit・Application golden fixture
- golden fixtureによる完全なKernelDecisionとContext digestの互換性検査
- 13地点のExplain Report・人向け表示・Human Authority状態の言語間互換性
- Human Authority、Contract反映、stale再分析、Challenge、Evidenceを再生するlifecycle golden scenario
- Action発行後に変わった入力の拒否と、書込みActionが明示した`output_refs`だけの受理
- 同じ入力に対する同じKernelDecision

### Rust Requirement assurance

Rust版のRequirement定義は、保証の強さを`assurance`で区別します。

```yaml
- id: contract-tests-passed
  phase: before-merge
  role: Builder
  result_schema: result.evidence
  assurance: evidence-backed
```

`assurance`を省略したRequirementは`attestation`です。この場合にKernelが保証するのは、指定Roleが発行済みContextに対して、現在も有効な根拠参照を持つ充足Resultを提出したことまでです。説明の意味的な正しさは保証しません。

`evidence-backed`は`result.evidence`だけに指定できます。Rust Kernelは、充足Resultが参照するEvidence Recordについて次を追加で要求します。

標準Ruleでは`data-evidence-recorded`と`distributed-effect-evidence-recorded`を`evidence-backed`とし、分析・設計・Challenge Requirementは`attestation`のままにしています。

- 対象Requirement InstanceとChangeが一致する
- `git_revision`が現在のRepository revisionと一致する
- `outcome`が`passed`である
- 対象に適用されるContractの全条項を`contract_clause_refs`で覆う
- `method`と、`artifact.uri`、`artifact.digest`、終了コード`artifact.exit_code: 0`がある

EvidenceはAction発行後に`Application::add_evidence`で追記し、Resultの`basis_refs`と`output_refs`から参照します。同じEvidence IDは上書きできません。

この段階で保証するのは「再現情報を持つ成功Evidence Recordが、現在revisionと条項に対応して記録されていること」です。EvidenceをCIが実際に生成したことまでは保証しません。そこまで保証する場合は、CI／runnerの署名と導入先Trust Storeによる検証を追加する必要があります。

### Rust signal applicability review

Result payload上の`not-applicable`は、Detector候補が実装に該当しないという判断を表します。signalが実在するがriskを受容する場合や、選択されたRequirementを省略する場合には使用しません。旧`excluded`保存値は受理しません。

Rust Kernelは`not-applicable`候補ごとに`risk-signal-applicability-reviewed` Instanceを生成し、独立したChallenger Actionを要求します。Challenger Contextの`not_applicable_signal_candidates`には、signal、binding、検出根拠参照、Analystの理由・根拠参照、Disposition Result IDが入ります。

- Challenger outcomeが`satisfied`: 非該当を確定する
- `unsatisfied`または`inconclusive`: 除外を支持せず、signalを`confirmed`としてRuleを評価する

Explainでは、確認前を`applicability-pending`、支持された後を`not-applicable`、支持されなかった後を`confirmed`と表示します。ResultはChangeごとに分離されます。candidate fingerprintはDetector ID・version、signal、bindingから生成し、根拠ref・digestは別のevidence versionとして保持します。`confirmed`は同じ論理候補なら再利用し、`not-applicable`は根拠digestが変わると再確認します。現行Rust PrototypeはRequirement省略を実装しておらず、`not-applicable`で代用することもできません。将来実装する場合は、Decision Record、決定権限、適用範囲、期限を持つ別経路にします。

### Rust Contract Health

`contract-health`は、全ChangeのResult・Evidenceと現在のRepository観測から、Contract条項ごとの実装準拠状態を再生成します。

- `verified`: 成功Evidenceを参照するBuilder outcomeの入力digestがすべて現在値と一致する
- `stale`: 検証履歴はあるが、Contract・コード・設定・Evidence等の入力が変わった、または失われた
- `unverified`: 検証履歴がない
- `failed`: 現在入力に対応するEvidenceが失敗または判断不能である

Contractへ状態を書き戻さず、未検証を合格扱いしません。

Applicationは、現在選択されたRequirementのsubjectと一致する`stale`／`failed`条項だけに、組込みの`contract-clause-revalidated` Requirementを追加します。これは`before-merge`の`evidence-backed`なBuilder作業です。Contextには対象条項とhealth findingを含め、無関係な条項と`unverified`条項はこの経路ではChangeを停止しません。現在入力に対する成功Evidenceを提出すると再検証は解決します。Repository全体を定期CIで停止する基準は、別の運用ポリシーとして扱います。

```sh
agentic-vnext-rust contract-health --project . --format text
agentic-vnext-rust contract-health --project . --format json --require-clean
```

## 実行

Repository rootで実行します。正準実装と受入テストはRust版です。`agentic_vnext/`のPython実装は過去の設計探索用参照であり、新しい設計の実装・同等性確認の対象にはしません。

Agentの通常利用経路はlocal stdio MCP serverです。同じRustバイナリを`mcp` subcommandで起動すると、`next`、`submit`、`explain`、`contract-health`と、発行Actionに限定されたEvidence、Decision、Contract書込みToolを利用できます。Tool契約と信頼境界は[`MCP-DESIGN.md`](MCP-DESIGN.md)、固定I/O Schemaは`schemas/mcp/v1/`にあります。既存CLIは人向け診断、CI、Release・binary管理の補助経路として残します。

```sh
agentic-vnext-rust mcp --project .
```

MCP serverは一つのProject rootへ固定され、stdoutをJSON-RPC専用にします。Action Resultは、同じsessionで`agentic_next`が発行した`change_id`、Action ID、Context digestの完全一致でのみ受理します。未提出Actionはprocess終了時に失効するため、再接続後は`agentic_next`を再実行してください。

```sh
sh tests/test-vnext.sh
```

Rust互換実装はContributor向けに別途実行します。通常のKit利用者へRust
toolchainを要求するものではありません。

```sh
sh tests/test-vnext-rust.sh
```

Rust CLIの個別確認は次のとおりです。

```sh
cargo run --manifest-path prototype/vnext/rust/Cargo.toml --locked -- \
  verify-canonicalization prototype/vnext/golden/v1
cargo run --manifest-path prototype/vnext/rust/Cargo.toml --locked -- \
  verify-schema prototype/vnext/golden/v1
cargo run --manifest-path prototype/vnext/rust/Cargo.toml --locked -- \
  verify-rules prototype/vnext/golden/v1
cargo run --manifest-path prototype/vnext/rust/Cargo.toml --locked -- \
  verify-detection prototype/vnext/golden/v1
cargo run --manifest-path prototype/vnext/rust/Cargo.toml --locked -- \
  verify-kernel prototype/vnext/golden/v1
cargo run --manifest-path prototype/vnext/rust/Cargo.toml --locked -- \
  verify-context prototype/vnext/golden/v1
cargo run --manifest-path prototype/vnext/rust/Cargo.toml --locked -- \
  verify-project prototype/vnext/golden/v1
cargo run --manifest-path prototype/vnext/rust/Cargo.toml --locked -- \
  verify-lock prototype/vnext/golden/v1
cargo run --manifest-path prototype/vnext/rust/Cargo.toml --locked -- \
  verify-submission prototype/vnext/golden/v1
cargo run --manifest-path prototype/vnext/rust/Cargo.toml --locked -- \
  verify-application prototype/vnext/golden/v1
cargo run --manifest-path prototype/vnext/rust/Cargo.toml --locked -- \
  verify-store prototype/vnext/golden/v1
cargo run --manifest-path prototype/vnext/rust/Cargo.toml --locked -- \
  verify-persistent prototype/vnext/golden/v1
cargo run --manifest-path prototype/vnext/rust/Cargo.toml --locked -- \
  verify-explain prototype/vnext/golden/v1
```

実際の導入Projectに対しては、Framework lockの`framework_release`に対応するReleaseを`.agentic/cache/releases/<release-id>/`から自動解決します。

```sh
cargo run --manifest-path prototype/vnext/rust/Cargo.toml --locked -- \
  next change.example \
  --project /path/to/project

cargo run --manifest-path prototype/vnext/rust/Cargo.toml --locked -- \
  explain change.example \
  --project /path/to/project \
  --format json
```

offline bundle等のReleaseを直接検証する場合だけ`--release <release-root>`を指定できます。既定は人向けtextです。`--format json`は`schemas/outputs/v1/`の固定形式を返します。ローカルの`next`と`explain`は未commitのtracked artifactも現在の内容で評価します。CIと同じclean checkoutを要求する場合は`--require-clean`を付けます。このmodeでは全Record、config、Framework lock、署名済みReleaseで使う公開鍵・取得元設定がGit管理対象であることも確認します。

Release rootには次を置きます。

```text
release.yaml
rules.yaml
schemas/v1/
```

署名済みRelease manifest v2は、Release ID、取得元ID、assetの相対path、全fileのSHA-256、署名鍵ID、Ed25519署名を持ちます。Framework lock v2はmanifestの署名対象部分のdigest、取得元ID、署名鍵IDを固定します。導入Projectは公開鍵と、その鍵に許可する取得元IDを`.agentic/trusted-release-keys.yaml`でGit管理します。`source_id`はURLやlocal pathではなく、配布経路を表す安定した論理IDです。

公開鍵設定v2は鍵ごとに`active`、`retired`、`revoked`を指定します。`active`だけが新しいReleaseのinstall・switchに使えます。`retired`は既存Releaseの通常実行とrollbackだけに使えます。`revoked`は既にcacheへ導入済みのReleaseも含めて拒否します。rotation時は新旧鍵を一時的に`active`で併存させ、新鍵Releaseへの切替後に旧鍵を`retired`へ変更します。rollback期間終了後、または鍵侵害時に`revoked`へ変更します。

新しいReleaseは、候補lockを有効化する前に導入します。

```sh
AGENTIC_RELEASE_SIGNING_KEY_HEX=<64-character-ed25519-seed> \
agentic-vnext-rust release build /path/to/release-source \
  --lock /path/to/base-framework.lock \
  --source-id remote:official \
  --key-id framework.release.2026 \
  --expected-public-key <64-character-ed25519-public-key> \
  --output /path/to/prototype-vnext-dev.tar \
  --lock-output /path/to/candidate-framework.lock \
  --format json

agentic-vnext-rust release fetch /path/to/candidate-framework.lock \
  --project /path/to/project

agentic-vnext-rust release install /path/to/offline-bundle \
  --lock /path/to/candidate-framework.lock \
  --project /path/to/project

agentic-vnext-rust release install-archive /path/to/prototype-vnext-dev.tar \
  --lock /path/to/candidate-framework.lock \
  --project /path/to/project

agentic-vnext-rust release switch /path/to/candidate-framework.lock \
  --project /path/to/project

agentic-vnext-rust release rollback /path/to/project/.agentic/cache/framework-lock-backups/<digest>.yaml \
  --project /path/to/project
```

`build`は入力directoryを変更せず、既存の`release.yaml`を除いた全fileを列挙して署名済みmanifestを生成します。file順、tar metadata、mtime、uid、gid、modeを固定するため、同じ入力・base lock・署名鍵・取得元ID・鍵IDから同じtarと候補lockを生成します。出力済みfileは上書きしません。

秘密鍵seedは固定名の環境変数`AGENTIC_RELEASE_SIGNING_KEY_HEX`からだけ読み、CLI引数、manifest、Framework lock、標準出力へ記録しません。`--expected-public-key`を指定すると、秘密鍵から導出した公開鍵が事前登録値と異なる場合は出力前に停止します。JSON receiptはRelease ID、manifestのartifact digest、tar全体のarchive digest、公開鍵、出力pathを返します。

Release候補のCIは次のように実行します。

```sh
AGENTIC_RELEASE_SIGNING_KEY_HEX=<64-character-ed25519-seed> \
AGENTIC_RELEASE_SIGNING_PUBLIC_KEY_HEX=<64-character-ed25519-public-key> \
AGENTIC_RELEASE_SOURCE_ID=remote:official \
AGENTIC_RELEASE_SIGNER_KEY_ID=framework.release.2026 \
sh prototype/vnext/scripts/release-ci.sh
```

このscriptは同じ入力を独立に二度buildしてtarと候補lockをbyte単位で比較し、それぞれを`release install-archive`で再検証します。その後に最終成果物を`dist/vnext/`へ作り、同じ検査をもう一度通します。`.github/workflows/vnext-release.yml`は手動起動だけを許可し、秘密鍵をRepository secret、対応する公開鍵をRepository variableから読み、検証済みFramework候補を14日間のCI Artifactとして保存します。外部Releaseへの公開や導入先lockの更新は行いません。

同じworkflowの秘密鍵を持たないmatrix jobは、次のnative binaryを同一commitからbuildします。

| OS・CPU | Rust target |
|---|---|
| Linux x64 | `x86_64-unknown-linux-gnu` |
| Linux arm64 | `aarch64-unknown-linux-gnu` |
| macOS Intel | `x86_64-apple-darwin` |
| macOS Apple Silicon | `aarch64-apple-darwin` |
| Windows x64 | `x86_64-pc-windows-msvc` |

各binaryには、source revision、target、Rust version、size、SHA-256を持つ`<binary>.build.json`を付けます。全5件が揃った場合だけ、決定的な`SHA256SUMS`とともに`agentic-vnext-release-binaries` Artifactへまとめます。各実行binaryにはGitHub Artifact Attestationも生成し、workflow、Repository、source revisionとbinary digestを結び付けます。

正式公開は別の`.github/workflows/vnext-publish-release.yml`を手動起動し、候補workflowのrun IDと`framework-<release_id>`形式のtagを指定します。このworkflowは候補runが同一Repository・既定branch・候補生成workflowの成功runであることを確認し、Artifactを公開前と承認後の二回downloadして再検証します。公開jobだけが`contents: write`を持ち、署名秘密鍵は受け取りません。

Repositoryには`vnext-release` Environmentを作成し、required reviewer、self-review禁止、既定branchだけを許可するdeployment branch ruleを設定してください。`AGENTIC_RELEASE_SIGNING_PUBLIC_KEY_HEX`、`AGENTIC_RELEASE_SOURCE_ID`、`AGENTIC_RELEASE_SIGNER_KEY_ID`はRepository variableとして管理します。Environment保護が設定されていなければ、workflowファイルに`environment`と書くだけでは人手承認を強制できません。

公開jobはFramework候補と5種類のbinary候補を同じrun IDから取得します。binaryについてはSHA-256、build record、source revisionに加え、候補生成workflowがGitHub-hosted runnerで作ったattestationであることを検証します。その後GitHub Releaseをdraftとして作成し、全assetを再downloadしてbyte単位で照合してから公開します。

```text
framework-release.tar
candidate-framework.lock
publish-receipt.json
SHA256SUMS
agentic-vnext-rust-<target>[.exe]
agentic-vnext-rust-<target>[.exe].build.json
publication-record.json
```

`publication-record.json`は候補run ID、source revision、取得元・署名鍵ID、Framework assetとbinary assetのdigestを記録する公開時の来歴情報です。Release署名やattestationの信頼元ではなく、どのCI runから公開したか追跡するための記録です。upload後検証に失敗した場合はdraftのまま停止し、自動削除や上書きを行いません。公開workflowもTrust Storeや導入先Framework lockは更新しません。

利用者はchecksumに加え、GitHub CLIでbinaryのprovenanceを検証できます。

```sh
gh attestation verify agentic-vnext-rust-x86_64-unknown-linux-gnu \
  --repo <owner>/<repository> \
  --signer-workflow <owner>/<repository>/.github/workflows/vnext-release.yml
```

公開済みbinaryの初回導入は、Release tagを明示してbootstrapを実行します。POSIX環境では次のとおりです。

```sh
sh prototype/vnext/bootstrap/install.sh \
  --repo <owner>/<repository> \
  --tag framework-<release-id> \
  --install-root "$HOME/.local/share/agentic"
```

WindowsではPowerShell版を使います。

```powershell
.\prototype\vnext\bootstrap\install.ps1 `
  -Repository <owner>/<repository> `
  -Tag framework-<release-id> `
  -InstallRoot "$env:LOCALAPPDATA\Agentic"
```

どちらもGitHub CLIを必要とします。Releaseのsource revisionと既定branchを取得し、Repository、候補生成workflow、source revision、source ref、GitHub-hosted runnerを固定してArtifact Attestationを検証してからbinaryを実行します。checksumだけを信頼するmodeや検証省略optionはありません。

検証済みbinaryは、導入先の`releases/<tag>/`へ不変のassetを保存し、`active`という2行の機械管理fileに現在tagと直前tagを記録します。`bin/agentic`または`bin/agentic.cmd`は現在tagのbinaryを起動します。CLI binaryの更新とProjectごとのFramework Release更新は別操作です。

```sh
agentic binary status --install-root /path/to/agentic

agentic binary update /path/to/downloaded-assets \
  --tag framework-<new-release-id> \
  --source-revision <40-character-git-sha> \
  --install-root /path/to/agentic

agentic binary rollback --install-root /path/to/agentic
```

bootstrapは公開assetの取得とattestation検証を含みます。`binary update`へdirectoryを直接渡す場合は、呼出し側が同じattestation検証を済ませる必要があります。binary manager自身はdirectoryのfile間整合性を再検証しますが、GitHubへ接続してattestationを取得しません。

remote取得元は`.agentic/release-sources.yaml`でGit管理します。

```yaml
schema_version: "1"
sources:
  - id: remote:official
    base_url: https://releases.example.com/agentic
```

`fetch`は候補lockの`source_id`からbase URLを選び、`<base_url>/<release-id>.tar`を取得します。通常はHTTPSだけを許可し、HTTPはlocal test用のloopback addressに限定します。redirectは追跡せず、downloadを64 MiB、展開後を256 MiB、archive entryを4096件に制限します。tar内の絶対path、`..`、symlink、hard link、特殊file、重複pathを拒否します。

`fetch`が安全に展開したdirectoryも信頼せず、`install`と同じ署名、取得元、署名対象manifestのdigest、全fileのdigest、Rule・Schema digest検証へ渡します。一時directoryを再検証後にrenameし、失敗時は一時directoryを除去して使用中ReleaseとFramework lockを変更しません。`switch`は導入済みReleaseと候補lockの全protocolを再検証し、現行lockをcacheへ退避してから候補lockを原子的に置き換えます。`rollback`も退避lockが参照するReleaseを再検証してから戻します。

主なファイルは次のとおりです。

| ファイル | 役割 |
|---|---|
| `rust/src/python_detection.rs` | Python構文から関数・呼出先・物理resourceを機械的に観測 |
| `rust/src/git_repository.rs` | Git解析対象の列挙、Binding Record適用、coverage生成 |
| `rust/src/detection.rs` | 正規化したrepository factからSignal候補を生成 |
| `rust/src/rules.rs` | Requirement・Ruleの構造検査とRule Index生成 |
| `rust/src/kernel.rs` | Requirement選択、freshness、次状態を判定する純粋ロジック |
| `rust/src/context.rs` | NextActionから実行用Contextと参照digestを生成 |
| `rust/src/project_runtime.rs` | 実Projectのconfig、Release、Git観測、Storeを接続 |
| `schemas/v1/` | 保存Recordの言語非依存Schema |
| `schemas/mcp/v1/` | Agent用MCP Toolの固定I/O Schema |
| `schemas/outputs/v1/` | 保存しない生成物の公開形式。Next Response v1とExplain Report v1 |
| `schemas/delivery/v1/` | 移行互換用の未署名Framework Release manifest |
| `schemas/delivery/v2/` | 署名済みRelease、鍵statusを持つ公開鍵設定、取得元、Framework lock拡張、Publish Receipt、Binary Build Record、Publication Recordの固定形式 |
| `golden/v1/` | canonical JSON、Schema、Kernel、Application、永続lifecycle、Explain Report等の固定期待値 |
| `rust/` | build済みバイナリ移行を検証するRust crate。Project loader、ProjectStore、Application、CLI、local stdio MCP serverを含む |
| `agentic_vnext/application.py` | `next`と`submit`のModule呼出順を管理 |
| `fixtures/db-sqs/` | DB更新＋SQS送信の固定入力 |

## 現時点の制約

- InMemory Adapterはテスト用です。Filesystem StoreではChange、Contract、Decision、Result、Evidenceを保存しますが、発行済みAction自体は保存せず、正本から再生成します。
- 実Projectの通常経路では、Observation Schema v3に手書きの`facts`・`coverage`を置きません。Rust版が`analysis.roots`配下のGit上のPython sourceを列挙・解析して生成します。
- 現在の言語DetectorはPythonのみで、DB書込みは`insert`・`update`・`delete`、message送信は`publish`・`send_message`を観測します。Binding済みresourceへの他method呼出しは`unsupported-observation`として停止します。alias、動的dispatch、framework固有APIは今後のDetector追加対象です。
- Binding Recordはartifact内の関数名・物理resource名ごとに、論理ID、owner、承認Decisionを記録します。承認Decisionは`accepted`でなければならず、artifact・binding・承認Decisionの変更は検出根拠digestへ反映されます。
- ContextはRequirement単位に分離していますが、現在の最小単位はContract文書IDとコードartifact IDです。Contract clauseやコードsymbol単位の選択は未実装です。
- Rust CLIはFramework lockからlocal Releaseを自動解決して`next`と`explain`を実行します。決定的な署名済みRelease生成、offline directory・local tar・remote tarの導入、切替、rollback、5 Platformのnative binary build、checksum・attestation、候補Artifact保存、Environment承認後のGitHub Release公開、attestation必須bootstrap、versioned binary更新・rollbackは実装済みです。実際のGitHub-hosted workflowによる公開・導入の実証、SBOM、認証付きFramework取得、resume、複数mirror、過去のResult IDを指定した説明は未実装です。
- Framework lock v2はRelease artifact digest、取得元ID、署名鍵IDを固定します。鍵のrotation・retire・revoke規則は実装済みです。組織提供Releaseの合成、署名済み失効listのremote同期、透明性logは未実装です。
- Result payload SchemaはPrototype組込みの6種類に固定されています。Project・組織固有Schemaの追加方法とSchema migrationは未実装です。
- lifecycle goldenは、実装でコードartifact digestが変わっても設計工程を繰り返さずEvidenceへ進む正常系を固定しています。stale Actionの拒否や保存競合等の異常系scenarioは通常テストだけで、まだ言語間golden化していません。
- `canonical-json-v1`は整数だけを扱います。浮動小数点の言語間正規化は未定義のため拒否します。
- Rust crateはcanonical JSONからExplain Reportまでgolden互換であり、実Projectのconfig、Git artifact、Filesystem Storeを読む`next`／`explain` CLIを含みます。
- cacheはwrite-throughのみです。検証済みcache readによる高速化は未実装です。
- cleanなlocal Git cloneでのCI再現まで実装済みです。remote CI status、shallow clone、submodule、複数Repositoryは未実装です。
- build phaseと解析root、Binding RecordはProject manifestへ明示します。risk factとcoverageはRust AdapterがGitとsourceから生成します。
- MCP Adapterは発行済みActionに応じてResult、Evidence、Decision、Contractを書き込みます。Decision／Contract更新は`expected_digest`による楽観的lockを使い、新規作成では`null`を明示します。remote MCP、複数Projectを扱う単一process、未提出Actionのprocess再起動を跨ぐresumeは未実装です。
- 現行`bin/agentic`、Schema、Skill、導入処理の挙動は変更しません。

したがって、このPrototypeのResult形式やModule APIを互換性のある公開仕様として利用しないでください。

## 配布実装との関係

広く配布するvNextでは、KernelとCLIをRustのbuild済みバイナリとして提供し、通常利用者にはPythonもRust toolchainも要求しない案を第一候補とします。現在は決定的な署名済みRelease生成、実Projectに対する`next`と`explain`、offline・remote Releaseの検証、atomic install、lock切替・rollback、5 Platformのnative binary、checksum・attestation、候補Artifact保存、承認付きGitHub Release公開まで接続済みです。Publisherの秘密鍵利用は候補生成CIに限定し、公開jobと通常利用者は公開鍵だけを保持します。詳細は`FRAMEWORK-REVIEW.md` 14.12、14.21、14.22を参照してください。
