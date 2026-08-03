# vNext shadow prototype

`FRAMEWORK-REVIEW.md` 14章のThin Kernel仮説を検証する実験実装です。公開APIや現行CLIの標準経路にはまだ切り替えていません。現行Projectを変更せずに移行対象を調べ、Migration Draftのレビュー検証後に隔離された候補を生成します。Completion Record、Framework Release署名、vNext Schemaを検証し、有効な候補を明示操作で既存Projectへ適用する経路まで接続しています。

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
- RustのPython・Java・Kotlin・Go・Rust・Ruby・PHP・C#・Swift・Scala・C・GDScript・JavaScript・JSX・TypeScript・TSX構文解析によるDB書込み・message publishの観測
- Git管理された解析rootの宣言漏れ、parse失敗、binding未解決、未対応観測のfail-closed化
- 未知のrepository fact kindの拒否
- versionedな標準Signal Domain Catalogと、未知Signal・不正binding参照のRule compile拒否
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
- 読取り時digestと排他lockによるShared Contractのstale更新拒否、および別条項の安全な並行更新
- 削除・破損から再生成できる`.agentic/cache/`へのwrite-through
- cleanな実Git cloneからの`ready-to-merge`再現
- Git revision、tracked artifact、未commit変更を検査するCI Evaluator
- 人向け本文を保持するChange・Contract・DecisionのMarkdown Record
- typed blockだけを更新し、YAML版と同じSnapshotを生成するMarkdown codec
- Change・Contract・Decision・Result・Evidenceの言語非依存JSON Schema検証
- 6種類のResult payload Schemaと、Result種別ごとの許可Role検証
- outcomeの結論・根拠参照と、発行Contextに対する参照整合性の検証
- 全ChangeのEvidence履歴と現在の入力digestから生成する条項単位のContract Health表示と、明示policyによる定期CIゲート
- 主要8 ORM・8 messaging・8 HTTP client・3 Object Storage SDK系統を測るDetector品質benchmark
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

標準Ruleでは`data-evidence-recorded`、`distributed-effect-evidence-recorded`、`security-evidence-recorded`を`evidence-backed`とし、分析・設計・Challenge Requirementは`attestation`のままにしています。Security Signalは、build前のoperation境界・Contract確認と設計Challenge、build後の実行証拠・実装Challengeを要求します。

- 対象Requirement InstanceとChangeが一致する
- `git_revision`が現在のRepository revisionと一致する
- `outcome`が`passed`である
- 対象に適用されるContractの全条項を`contract_clause_refs`で覆う
- `method`と、`artifact.uri`、`artifact.digest`、終了コード`artifact.exit_code: 0`がある

EvidenceはAction発行後に`Application::add_evidence`で追記し、Resultの`basis_refs`と`output_refs`から参照します。同じEvidence IDは上書きできません。

この段階で保証するのは「再現情報を持つ成功Evidence Recordが、現在revisionと条項に対応して記録されていること」です。EvidenceをCIが実際に生成したことまでは保証しません。そこまで保証する場合は、CI／runnerの署名と導入先Trust Storeによる検証を追加する必要があります。

### Standard Signal Domains

組込みCatalog v3は、標準Signalを`data-persistence`、`distributed-integration`、`security-boundary`の3 domainに整理します。domainは分類用metadataであり、Ruleは従来どおり個別のSignal IDを指定します。domainを指定した一括適用や、domain名からの意味推測は行いません。

```sh
agentic catalog signal-domains --format text
agentic catalog signal-domains --format json
```

`db_write`／`message_publish`／`external_call`／`object_write`からSignalへの変換も同じCatalogで表駆動にし、Detector本体の言語・framework別分岐から分離しました。組込み定義は所有型の`SignalCatalogRegistry`へ読み込み、Git Repository Adapter、Rule Compiler、Detectorは実Projectが保持する同じRegistry instanceを参照します。JSON出力は`schemas/catalog/v1/signal-domain-catalog.schema.json`に従い、canonical digestを含みます。対応表、Registry境界、追加条件、意図的に未収録の候補は[`SIGNAL-DOMAINS.md`](SIGNAL-DOMAINS.md)に記載しています。

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

Applicationは、現在選択されたRequirementのsubjectと一致する`stale`／`failed`条項だけに、組込みの`contract-clause-revalidated` Requirementを追加します。これは`before-merge`の`evidence-backed`なBuilder作業です。Contextには対象条項とhealth findingを含め、無関係な条項と`unverified`条項はこの経路ではChangeを停止しません。現在入力に対する成功Evidenceを提出すると再検証は解決します。

```sh
agentic contract-health --project . --format text
agentic contract-health --project . --format json --require-clean
```

通常の`contract-health`は診断用なので、状態にかかわらず終了code 0です。Repository全体の定期CIでは、停止対象をproject所有のpolicyへ明示し、`--policy`でゲートを有効にします。

```yaml
# .agentic/contract-health-policy.yaml
schema_version: "1"
fail_on:
  - failed
  - stale
```

```sh
agentic contract-health \
  --project . \
  --policy .agentic/contract-health-policy.yaml \
  --format json \
  --require-clean
```

`fail_on`に指定できるのは`failed`、`stale`、`unverified`です。該当条項があれば、固定形式のGate Reportをstdoutへ出したうえで終了code 1、なければ0を返します。`verified`は停止対象ではなく、空のpolicyや未知状態も受理しません。`--require-clean`ではpolicyもGit追跡済みでなければなりません。入力Schemaは`schemas/ci/v1/contract-health-policy.schema.json`、出力Schemaは`schemas/outputs/v1/contract-health-gate-report.schema.json`です。

検証済みの`agentic` binaryをrunnerへ導入済みなら、GitHub Actionsでは通常のChange CIと分離して次のように定期実行できます。

```yaml
name: Contract Health
on:
  workflow_dispatch:
  schedule:
    - cron: "17 2 * * 1"
permissions:
  contents: read
jobs:
  contract-health:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - run: >-
          agentic contract-health
          --project .
          --policy .agentic/contract-health-policy.yaml
          --format json
          --require-clean
```

### Rust Detector Benchmark

`benchmark`は、review済みの正解データと明示閾値を持つ固定corpusに対して、source observationとframework candidateのprecision／recallを測ります。浮動小数点や実行時間を合否に使わず、0〜10000のbasis pointで決定的に集計します。

同梱の`major-frameworks-v1`は、主要8 ORM、8 messaging framework、8 HTTP client、Amazon S3・Google Cloud Storage・Azure Blob Storageを扱う10個のproject fixtureです。外部sourceのコピーではなく、各frameworkの典型的な呼出し形とmanifest・import・型・receiver根拠を再現する最小corpusです。

`real-projects-v1`は、django-oscar、Prisma Examples、NATS Go、Godot Demo Projectsから、ライセンスを保持した代表sourceと依存manifestを固定Git revisionで収録したoffline corpusです。8 sourceに含まれる31 receiver callと9 framework candidateを人が原文と照合し、期待値として固定しています。候補だけをreviewするcaseは`reviewed_outputs: [framework_candidates]`を明示し、未reviewの全receiver callをprecision／recallへ混ぜません。Djangoの明示的なcollection更新と、Prismaを使わないService Worker・Drizzle・TypeORM sourceをnegative caseとして含めます。

```sh
agentic benchmark \
  testdata/benchmarks/major-frameworks-v1 \
  --format text

agentic benchmark \
  testdata/benchmarks/real-projects-v1 \
  --format text
```

現在の基準は、SQLAlchemy `session.execute`とJavaScript版S3 `client.send`のようにkindを自動決定しない呼出しを含む32 receiver callと29 framework candidateのprecision／recallがすべて100%です。構文エラーは常に失敗し、corpusの`minimum_scores`を下回ると詳細Reportをstdoutへ出して終了code 1になります。Reportはmanifest・source・dependency manifest全体の`corpus_digest`も返します。正解値・閾値をDetectorが自動更新することはありません。

corpus入力は`schemas/benchmarks/v1/detector-corpus.schema.json`、JSON Reportは`schemas/outputs/v1/detector-benchmark-report.schema.json`に従います。各projectは`authored-fixture`または`external-snapshot`のprovenance、license、外部snapshotならRepository URL、40桁のGit revision、同梱LICENSEへのpathを必須とします。LICENSEもcorpus digestへ含めます。runnerはoffline・read-onlyで、corpus root外へのpathやsymlink escapeを拒否します。実際の外部Repository snapshotを追加する場合も、sourceのライセンスとrevisionをreviewしたうえで、同じcorpus契約へ人が正解を記入します。

### Repository Detector Audit

`detector-audit`は、Git管理下にある登録済み拡張子のsourceをRepository全体で走査します。言語別・file別にparse結果、`db_write`、`message_publish`、未分類receiver call、framework candidate、明示method Bindingが必要な候補、空のsuggestionを集計します。JSONの`candidate_records`には、候補ごとのpath、symbol、resource、method、根拠、提案kindを保持し、人手review対象を再現できます。

```sh
agentic detector-audit /path/to/repository --format text --require-clean

agentic detector-audit-check /path/to/repository \
  --baseline testdata/benchmarks/repository-audits-v1/django-oscar.yaml \
  --format text
```

このReportは正解ラベルを持たないため、precision／recallや違反有無を推測しません。未対応言語、parse失敗、source・依存manifestの読込み失敗が1件でもあれば`blocked`と終了code 1を返します。`--require-clean`はrevisionと解析bytesの対応を要求する再現検証用です。JSONは`schemas/outputs/v1/repository-detector-audit-report.schema.json`に従い、全source・読込み済み依存manifestから`content_digest`を生成します。

`detector-audit-check`は、固定revision、全入力の`content_digest`、候補詳細を含む監査Report全体のdigest、coverage gapをreview済みbaselineと比較します。baseline一致は「同じ監査結果を再現した」ことだけを表し、既知gapを通常評価で許可しません。Prisma baselineは一致しても`audit_status: blocked`を保持し、通常の`detector-audit`は終了code 1のままです。入力は`schemas/benchmarks/v1/repository-audit-baseline.schema.json`、出力は`schemas/outputs/v1/repository-audit-baseline-report.schema.json`に従います。

固定revisionの4 OSS cloneをRepository全体で監査した初回結果は次のとおりです。観測数と候補数はparse成功fileだけを集計します。

| Repository | source | parsed | parse gap | observations | framework candidates |
| --- | ---: | ---: | ---: | ---: | ---: |
| django-oscar | 836 | 836 | 0 | 16,387 | 766 |
| Prisma Examples | 652 | 651 | 1 | 2,785 | 344 |
| NATS Go | 179 | 179 | 0 | 26,429 | 648 |
| Godot Demo Projects | 497 | 497 | 0 | 5,833 | 0 |

初回監査で見つかったGodot Demo Projectsの2件は、Godotで有効な`$%UniqueNode`を同梱Tree-sitter文法が読めないことが原因でした。Godot公式の[GDScript reference](https://docs.godotengine.org/en/stable/tutorials/scripting/gdscript/gdscript_basics.html#literals)は`$NodePath`と`%UniqueNode`を定義し、固定revisionの公式demoでも[`$%SDFGI`](https://github.com/godotengine/godot-demo-projects/blob/4652e17c04fe5f249dc53949fb195a3d8b24ee5f/3d/truck_town/car_select/car_select.gd#L12)が使われています。Detectorは解析時だけ同じbyte長の互換表現を使い、報告する行番号とresource文字列には原文を維持します。修正後は固定revisionの497件をすべてparseできました。

Prisma Examplesの1件は、同一の`const db`初期化が未完了の式の途中へ重複している[上流source](https://github.com/prisma/prisma-examples/blob/eb8f4328821c6746680a2ba02e0e5636a085a327/databases/kysely-prisma-postgres/src/index.ts#L27-L41)自体の構文不整合です。このgapはDetectorで読み替えず、file path付きの`parse-error`としてfail closedを維持します。正当な新構文、上流source自体の構文不整合、生成途中fileなどの区別は人が確認し、Detectorまたは監査対象の期待値へ明示的に反映します。

候補reviewでは、nested manifestのFramework根拠がmonorepoの兄弟projectへ漏れる問題と、Prisma CLIだけを使うDrizzle projectをPrisma Clientと誤認する問題が見つかりました。manifest根拠はそのmanifestのdirectory配下だけへ適用し、Prismaのproject根拠はruntime packageの`@prisma/client`に限定します。さらにPrisma candidateは`prisma`を含むreceiverまたは明示的な`new PrismaClient()` aliasを要求します。Djangoの`__dict__`やsource上でcollection生成が明らかなreceiverの`update`も候補から除外します。4 OSSのbaselineは`benchmarks/repository-audits-v1/`へ固定しています。

## 実行

Repository rootで実行します。正準実装と受入テストはRust版です。`legacy/agentic_vnext/`のPython実装は過去の設計探索用参照であり、新しい設計の実装・同等性確認の対象にはしません。

### 現行Projectの移行診断

現行CLIを導入済みのRepositoryでは、初期化やfile変換の前に次のcommandを実行します。Gitのtop-levelを対象とし、固定revisionで比較できるようcleanなworktreeを要求します。

```sh
agentic migration inspect \
  --project /path/to/current-project \
  --format text

agentic migration inspect \
  --project /path/to/current-project \
  --format json
```

このcommandは、現行CLIのinstallation metadata、config、Contract、Decision、Change workflow、Evidenceを列挙し、次の区分で移行対象を示します。

- `mechanical`: pathや件数など、意味判断なしで復元できる情報
- `review-required`: Contract、Decision、Policy、Evidenceなど、人が意味を対応付ける情報
- `generated`: 対象revisionからDetectorで生成し、reviewする情報
- `release-supplied`: 署名済みFramework Releaseから導入する情報
- `already-present`: vNext形式で既に存在する情報

`readiness`は`review-required`、`blocked`、`already-vnext`のいずれかです。現行configとvNext activation fileが混在する場合、worktreeがdirtyな場合、必要なroot・metadataが読めない場合は`blocked`になります。診断Reportを生成できた場合は、`blocked`でもcommand自体は成功します。Gitまたはfilesystemを安全に読めずReportを生成できない場合だけ非0で終了します。

診断はProject fileを作成・更新・削除せず、stageやcommitも行いません。JSON形式は`schemas/outputs/v1/migration-inspection-report.schema.json`に従います。

診断結果が`review-required`の場合は、同じclean revisionからMigration Draftを生成できます。

```sh
agentic migration draft \
  --project /path/to/current-project \
  --format text

agentic migration draft \
  --project /path/to/current-project \
  --format json
```

Migration Draft v2は、移行対象ごとに元のpath、移行先、処理区分、人のreviewが必要か、完了確認を示す作業票です。現行configやContractの意味を自動変換せず、次の処理区分を使います。

- `inventory-only`: 移行入力を列挙するだけで、fileを変換しない
- `replace-after-review`: 現行の意味をreviewし、vNextの別形式へ置き換える
- `transform-after-review`: 対応関係を人が決めた後で変換する
- `generate-from-revision`: 固定revisionからDetector出力を生成してreviewする
- `install-from-release`: review済みの署名済みFramework Releaseから導入する

診断結果が`blocked`の場合は、指摘を解消して`migration inspect`をやり直します。`already-vnext`の場合はDraftを生成しません。Draft生成もProject file、Git index、commitを変更しません。JSON形式は`schemas/outputs/v1/migration-draft.schema.json`に従います。

生成直後の各`action.review`は`null`です。人が元のContractやDecisionなどを確認し、`requires_human_review: true`のactionだけに次の項目を記入します。CLIは意味を推測して補完しません。

- `decision`: `proceed`、`retire`、またはDecision・Change workflowで使える`preserve-history`
- `reviewer`: レビュー担当者を識別できる値
- `rationale`: 判断理由
- `evidence_refs`: 判断根拠をたどれる一意な参照

記入後は、生成済みフィールドとsource revisionを変更していないことを検証します。

```sh
agentic migration validate-draft \
  --project /path/to/current-project \
  --draft .agentic/migration-draft.json \
  --format text
```

検証結果は`valid`、`invalid`、`blocked`のいずれかです。`valid`になるには、必要なreviewがすべてそろい、生成時とGitのHEADが一致し、Draft以外のworktreeがcleanでなければなりません。DraftをRepository内へ保存した場合、指定したDraft fileだけはdirty判定から除外します。それ以外の変更は除外しません。検証もfileを変更せず、JSON形式は`schemas/outputs/v1/migration-draft-validation-report.schema.json`に従います。

`valid`なDraftから、既存Projectとは別のdirectoryへ候補Bundleを生成できます。出力先は`.agentic/migration-candidates/<name>`配下に限定され、既存のfileやdirectoryがある場合は上書きしません。

```sh
agentic migration generate-candidate \
  --project /path/to/current-project \
  --draft .agentic/migration-draft.json \
  --output .agentic/migration-candidates/review-1 \
  --format text
```

候補Bundleには次の3 fileを生成します。

- `.agentic/config.yaml`: vNextの標準pathを使う設定候補
- `migration-draft.json`: 検証済みDraftの正規化した写し
- `migration-manifest.yaml`: source revision、Draft digest、生成fileのdigest、未完了作業

Migration DraftだけではContractやDecisionの具体的な変換内容、Detector出力、署名済みFramework Releaseを確定できません。これらを自動生成せず、`proceed`と判断された項目をManifestの`pending_actions`へ残します。`preserve-history`も履歴保存が終わるまで未完了です。`retire`は生成対象にしません。そのため、生成直後の候補は常に`incomplete`であり、vNext Projectとして適用できるとは扱いません。

Manifest v2は`schemas/outputs/v1/migration-candidate-manifest.schema.json`に従います。候補生成では、指定した新規directoryと不足している親directoryだけを作成します。現行の`.agentic/config.yaml`、Contract、Decision、Git index、commitは変更しません。

候補生成後は、次のcommandで生成時の整合性と未完了作業を検証できます。

```sh
agentic migration validate-candidate \
  --project /path/to/current-project \
  --candidate .agentic/migration-candidates/review-1 \
  --format text
```

この検証では、source revision、埋め込まれたDraftのdigestとレビュー、Manifestの生成済みフィールド、vNext設定候補とDraft fileのbyte digestを確認します。候補directoryと、同じDraft digestを持つ生成元Draftだけをworktreeのdirty判定から除外します。それ以外の変更があれば`blocked`です。

生成直後の結果は、整合性に問題がなければ`incomplete`です。`pending_actions`が残るためcommandは非0で終了します。生成fileやManifestの改変は`invalid`、source側の検証を妨げる状態は`blocked`です。JSON Report v2は`schemas/outputs/v1/migration-candidate-validation-report.schema.json`に従います。

候補にfileを追加しただけでは`pending_actions`を完了扱いにしません。各actionについて、`migration-completions/<action-id>.yaml`へCompletion Recordを作成します。形式は`schemas/outputs/v1/migration-action-completion.schema.json`で定義しています。

```yaml
schema_version: "1"
kind: migration-action-completion
action_id: contracts
source_revision: <Manifestと同じGit revision>
draft_digest: <Manifestと同じDraft digest>
review:
  reviewer: migration-reviewer
  rationale: Reviewed every migrated Contract against its source.
  evidence_refs:
    - review:contracts-migration
completed_checks:
  - <埋め込まれたDraftのcompletion_checksを省略せず記載>
artifacts:
  - path: contracts/checkout.yaml
    digest: sha256:<file bytesのSHA-256>
```

検証時は、Recordのsource revisionとDraft digest、reviewの必須項目、Draftに固定された全completion check、成果物のpathとbyte digestを照合します。同じ成果物を複数actionから参照したり、生成済みManifestやDraftを成果物として申告したりすることはできません。Completion Recordに申告されていない候補fileも`invalid`です。`proceed`の成果物はactionのtarget path内、`preserve-history`の成果物は`migration-history/<action-id>/`配下に限定されます。Framework Release cacheは署名済みfile inventoryとしてまとめて検証するため、Completion Recordへの列挙は不要です。

有効なCompletion Recordが確認されたactionは`pending_actions`から外れます。未完了のactionがある間は、`pending_validations`にも`candidate-schema-and-release`が表示されます。すべてのactionが完了すると、次の検証を実行します。

1. Candidate configとFramework lockを読み込む
2. Trust Storeの有効な鍵でFramework Releaseの署名、source、artifact digest、file inventoryを検証する
3. `rules.yaml`が選択した署名済みReleaseのRule sourceと一致することを確認する
4. Repository Observationを元Projectの固定revisionに対して再実行し、source coverageとBinding authorityを確認する
5. Completion Recordが申告した有効Recordの一覧と、実際に読み込めるRecordの一覧を照合する
6. Contract、Decision、Change、Evidence、Resultを選択したReleaseのvNext Schemaで検証する

すべて成功すると結果は`valid`になり、commandは終了コード0を返します。署名改変、Schema違反、未承認のBinding authorityなどがあれば`invalid`です。`valid`になった候補だけを、次の明示操作で既存Projectへ適用できます。

```sh
agentic migration apply-candidate \
  --project /path/to/current-project \
  --candidate .agentic/migration-candidates/review-1 \
  --format text
```

適用前に候補をもう一度検証し、検証結果が`valid`でなければProjectを変更せず終了します。適用時は次の順に処理します。

1. Manifestのdigestとsource revisionから一意なapplication IDを作り、同じ適用の重複実行を拒否する
2. 適用先を事前検査し、移行元として退避するpath以外に異なる内容のfileがあれば上書きせず終了する
3. 現行config、Contract、Decision、Change workflow、Evidenceを`.agentic/migration-history/<application-id>/source/`へ元の相対pathのまま退避する
4. Candidateの設定、review済み成果物、署名済みFramework ReleaseをProjectの有効pathへ配置する
5. Manifest、Draft、Completion Record、適用結果を`.agentic/migrations/<application-id>/`へ保存する

途中で失敗した場合は、作成したfileを削除し、退避した移行元を元のpathへ戻します。自動復元にも失敗した場合は、エラーに復元失敗を明記します。Candidate自体は削除しません。Git indexとcommitも変更しないため、適用後は差分と退避内容を確認し、対象fileを`git add`してから通常のvNext検証を実行します。検証に成功した場合だけcommitしてください。

text出力とJSON出力には、適用したfileのdigest、退避元と退避先、次の操作を含めます。JSON形式は`schemas/outputs/v1/migration-application.schema.json`に従い、同じ内容を`<application-root>/application.yaml`へ保存します。

適用後の標準確認手順は次のとおりです。stage前に差分と`.agentic/migration-history/<application-id>/`の退避内容を確認してください。最初のBinding検証はstage済みfileを対象に実行し、commit後は`--require-clean`でもう一度確認します。

```sh
git add <reviewed-migration-paths>
agentic project validate-bindings --project /path/to/current-project --format json
git commit -m "migrate project to vNext"
agentic project validate-bindings --project /path/to/current-project --require-clean --format json
agentic next change.example --project /path/to/current-project --require-clean --format json
```

旧EvidenceがYAMLで保存されている場合も、vNextへ`proceed`するEvidence成果物は`.json`へ変換します。Filesystem Storeが有効Recordとして読み込むEvidenceはJSON fileであり、Completion Recordのartifact pathとdigestも変換後の`.json`を参照させます。旧YAMLは移行元としてApplication archiveに残ります。

公開済みbinaryをbootstrapした後、新しいGit Repositoryは次の順に初期化します。`project init`は既存fileを上書きせず、attestation検証済みbinaryと同じdirectoryに保存された候補から、config、Framework lock、Trust Store、Release cache、空のRepository Observationを作ります。解析rootを省略した場合はRepository全体を表す`.`です。

```sh
agentic project init --project /path/to/project
agentic change init change.example \
  --title "変更タイトル" \
  --intent "変更の意図" \
  --project /path/to/project
```

生成fileは自動でstage・commitしません。内容をreviewしてGitへ追加してください。sourceがある場合は、次のread-only commandで対応言語上の物理関数・resourceを列挙できます。C++などDetector未実装の主要言語も`detector_status: unsupported`としてinventoryへ残ります。

`project init`の完了時、Binding不足による`next`の停止時、および`project validate-bindings`の`invalid`時には、英語のhuman-readable出力で`project observe`からreview・再検証へ進む`Next:`を表示します。JSON出力は機械処理用Schemaを維持し、この案内を混入させません。候補が自動適用されないことも明記します。

```sh
agentic project observe \
  --project /path/to/project \
  --analysis-root src \
  --format yaml \
  --output .agentic/repository-observation.draft.yaml
```

この出力はRepository Observation Draft v6であり、Binding Recordの下書きです。`--output`はProject相対pathだけを受理し、既存file・symlinkを上書きしません。省略時は従来どおり標準出力へ返します。sourceごとのSHA-256を`source_digests`へ、生成時点の正式ObservationのSHA-256を`base_observation_digest`へ固定し、反映前のfreshness検査に使います。論理ID、owner、`authority_ref`を作りません。主要8 ORM・8 messaging frameworkに加え、Requests、HTTPX、明示receiver付きFetch、Axios、Java HttpClient、Spring WebClient、Go `net/http`、.NET HttpClientと、Amazon S3、Google Cloud Storage、Azure Blob Storageについて、project manifest・import・型名・receiver形状を根拠に`framework_candidates`を提示します。署名済み公式ReleaseのFramework Detection Catalogを使う場合は、TypeORMも候補化します。候補は常に`review_status: required`で、`suggested_fact_kinds`も非authoritativeです。明確なObject Storage uploadは、永続書込みと外部system呼出しの両面を表す`[external_call, object_write]`を提示します。

Draft v6の`binding_artifacts`は、Observation Schema v5の`artifacts`へ反映できる構造だけを機械的に作ります。観測に関係する物理symbol・resourceと、明示Bindingが必要なframework methodをキーにしますが、意味を持つ`fact_kinds`、`logical_refs`、owner、`authority_ref`は`null`のままです。不要な項目を除き、残すすべての`null`を既存コードと設計の調査結果、accepted Decisionに基づいて埋めるまで有効なBinding Recordにはなりません。`project observe`はproject fileを更新しません。

編集したDraftは、正式Observationへ反映する前に検査できます。Draft検査は未記入、物理名・論理ID・fact kindの不整合、未承認Decision、生成後のsource変更を停止対象にします。text出力だけを提供し、正式Observationやその他のProject fileは変更しません。

```sh
agentic project validate-bindings \
  --project /path/to/project \
  --draft .agentic/repository-observation.draft.yaml
```

検査に成功したDraftだけを、明示commandで正式Observationへ反映できます。反映直前にも同じ検査を行い、source、accepted Decision、生成元Observationのいずれかが変わっていれば何も変更しません。成功時は現在の`phase`を保ち、`analysis_roots`とreview済み`binding_artifacts`だけからObservation Schema v5を作り、設定済みfileを原子的に置換します。Draft自体は削除しません。

```sh
agentic project promote-bindings \
  --project /path/to/project \
  --draft .agentic/repository-observation.draft.yaml
```

反映後は`git diff`と`agentic project validate-bindings`で正式Observationを確認し、Observationとaccepted Decisionを一緒にcommitします。

転記・review後は、通常評価の前にBindingだけを検査できます。`invalid`は不足・曖昧・不正なBindingまたは未承認authority、`blocked`は未対応言語や構文エラーなど、完全なBinding検査を妨げるcoverage gapです。どちらも終了codeは非0です。`--require-clean`はCIで使用します。

```sh
agentic project validate-bindings \
  --project /path/to/project \
  --format json
```

JSON出力は`schemas/outputs/v1/binding-validation-report.schema.json`に従い、issueごとに`category: binding|coverage`、安定した`kind`、artifact ref、理由を返します。このcommandは論理ID、owner、kind、authorityを補完せず、既存の観測・Binding・Decisionだけを検証します。

```yaml
schema_version: "6"
kind: repository-observation-draft
analysis_roots: [shop]
base_observation_digest: sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
source_digests:
  shop/service.py: sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
artifacts:
  - path: shop/service.py
    framework_candidates:
      - framework: django-orm
        binding_key: order.save
        suggested_fact_kinds: [db_write]
        method_binding_required: true
        review_status: required
binding_artifacts:
  - ref: code.shop.service
    path: shop/service.py
    language: python
    bindings:
      symbols:
        place_order:
          logical_ref: null
          owner: null
          authority_ref: null
      resources:
        order:
          logical_refs: null
          owner: null
          authority_ref: null
      methods:
        order.save:
          fact_kinds: null
          owner: null
          authority_ref: null
```

SQLAlchemyの`execute`はSELECTとDMLの両方を受け取るため、`suggested_fact_kinds: []`で出力し、call単位の確認を要求します。候補生成は通常のrepository評価、coverage、fact生成には影響しません。

```yaml
bindings:
  symbols: {}
  resources: {}
  methods:
    session.execute:
      fact_kinds: [db_write]
      owner: team.ordering
      authority_ref: decision.repository-bindings
```

Agentの通常利用経路はlocal stdio MCP serverです。同じRustバイナリを`mcp` subcommandで起動すると、`next`、`submit`、`explain`、`contract-health`と、発行Actionに限定されたEvidence、Decision、Contract書込みToolを利用できます。Tool契約と信頼境界は[`MCP-DESIGN.md`](MCP-DESIGN.md)、固定I/O Schemaは`schemas/mcp/v1/`にあります。既存CLIは人向け診断、CI、Release・binary管理の補助経路として残します。

```sh
agentic mcp --project .
```

MCP serverは一つのProject rootへ固定され、stdoutをJSON-RPC専用にします。Action Resultは、同じsessionで`agentic_next`が発行した`change_id`、Action ID、Context digestの完全一致でのみ受理します。未提出Actionはprocess終了時に失効するため、再接続後は`agentic_next`を再実行してください。

```sh
sh scripts/tests/test-vnext.sh
```

Rust互換実装はContributor向けに別途実行します。通常のKit利用者へRust
toolchainを要求するものではありません。

```sh
sh scripts/tests/test-vnext-rust.sh
```

Rust CLIの個別確認は次のとおりです。

```sh
cargo run --locked -- \
  verify-canonicalization testdata/golden/v1
cargo run --locked -- \
  verify-schema testdata/golden/v1
cargo run --locked -- \
  verify-rules testdata/golden/v1
cargo run --locked -- \
  verify-detection testdata/golden/v1
cargo run --locked -- \
  verify-kernel testdata/golden/v1
cargo run --locked -- \
  verify-context testdata/golden/v1
cargo run --locked -- \
  verify-project testdata/golden/v1
cargo run --locked -- \
  verify-lock testdata/golden/v1
cargo run --locked -- \
  verify-submission testdata/golden/v1
cargo run --locked -- \
  verify-application testdata/golden/v1
cargo run --locked -- \
  verify-store testdata/golden/v1
cargo run --locked -- \
  verify-persistent testdata/golden/v1
cargo run --locked -- \
  verify-explain testdata/golden/v1
```

実際の導入Projectに対しては、Framework lockの`framework_release`に対応するReleaseを`.agentic/cache/releases/<release-id>/`から自動解決します。

```sh
cargo run --locked -- \
  next change.example \
  --project /path/to/project

cargo run --locked -- \
  explain change.example \
  --project /path/to/project \
  --format json
```

offline bundle等のReleaseを直接検証する場合だけ`--release <release-root>`を指定できます。既定は人向けtextです。`--format json`は`schemas/outputs/v1/`の固定形式を返します。ローカルの`next`と`explain`は未commitのtracked artifactも現在の内容で評価します。CIと同じclean checkoutを要求する場合は`--require-clean`を付けます。このmodeでは全Record、config、Framework lock、署名済みReleaseで使う公開鍵・取得元設定がGit管理対象であることも確認します。

Release rootには次を置きます。

```text
release.yaml
rules.yaml
framework-catalog.yaml  # 任意。署名済みRelease v2だけが指定可能
schemas/v1/
```

署名済みRelease manifest v2は、Release ID、取得元ID、assetの相対path、全fileのSHA-256、署名鍵ID、Ed25519署名を持ちます。Framework lock v2はmanifestの署名対象部分のdigest、取得元ID、署名鍵IDを固定します。導入Projectは公開鍵と、その鍵に許可する取得元IDを`.agentic/trusted-release-keys.yaml`でGit管理します。`source_id`はURLやlocal pathではなく、配布経路を表す安定した論理IDです。

Framework Releaseの開発者は、`framework-catalog.yaml`でFramework固有のmethod候補を追加できます。各ruleにはnamespace、対象言語、method、manifestまたはsourceの根拠、候補のfact kind、説明を記述します。外部Framework IDは`namespace/name`になるため、組込みruleとは衝突しません。`agentic` namespaceは組込みrule専用です。

導入先の開発者がProject内に任意のCatalogを追加する仕組みではありません。`project observe`が読むのは、activeなFramework lockから解決し、署名と全file digestを検証したRelease内のCatalogだけです。Catalogが生成するのは`review_status: required`の候補であり、Binding Recordへ自動反映されません。Catalogを書き換えたReleaseへ切り替える場合も、Release全体のinstallとlockのswitchが必要です。rollbackでは以前のlockとReleaseに含まれるCatalogへ戻ります。

公開鍵の初回信頼には`distribution-trust.json`を使います。このfileはRelease archiveやPublish Receiptとは独立してGitHub Artifact Attestationを付け、bootstrapが実行binaryと同じRepository、workflow、source revision、source refに固定して検証します。`publish-receipt.json`や`publication-record.json`内の公開鍵だけを信頼起点にはしません。

公開鍵設定v2は鍵ごとに`active`、`retired`、`revoked`を指定します。`active`だけが新しいReleaseのinstall・switchに使えます。`retired`は既存Releaseの通常実行とrollbackだけに使えます。`revoked`は既にcacheへ導入済みのReleaseも含めて拒否します。rotation時は新旧鍵を一時的に`active`で併存させ、新鍵Releaseへの切替後に旧鍵を`retired`へ変更します。rollback期間終了後、または鍵侵害時に`revoked`へ変更します。

新しいReleaseは、候補lockを有効化する前に導入します。

```sh
AGENTIC_RELEASE_SIGNING_KEY_HEX=<64-character-ed25519-seed> \
agentic release build /path/to/release-source \
  --lock /path/to/base-framework.lock \
  --source-id remote:official \
  --key-id framework.release.2026 \
  --expected-public-key <64-character-ed25519-public-key> \
  --framework-catalog framework-catalog.yaml \
  --output /path/to/prototype-vnext-dev.tar \
  --lock-output /path/to/candidate-framework.lock \
  --format json

agentic release fetch /path/to/candidate-framework.lock \
  --project /path/to/project

agentic release install /path/to/offline-bundle \
  --lock /path/to/candidate-framework.lock \
  --project /path/to/project

agentic release install-archive /path/to/prototype-vnext-dev.tar \
  --lock /path/to/candidate-framework.lock \
  --project /path/to/project

agentic release switch /path/to/candidate-framework.lock \
  --project /path/to/project

agentic release rollback /path/to/project/.agentic/cache/framework-lock-backups/<digest>.yaml \
  --project /path/to/project
```

`build`は入力directoryを変更せず、既存の`release.yaml`を除いた全fileを列挙して署名済みmanifestを生成します。`--framework-catalog`を指定した場合は、Schema、namespace、言語、重複rule、fact kindを出力前に検証します。file順、tar metadata、mtime、uid、gid、modeを固定するため、同じ入力・base lock・署名鍵・取得元ID・鍵IDから同じtarと候補lockを生成します。出力済みfileは上書きしません。

秘密鍵seedは固定名の環境変数`AGENTIC_RELEASE_SIGNING_KEY_HEX`からだけ読み、CLI引数、manifest、Framework lock、標準出力へ記録しません。`--expected-public-key`を指定すると、秘密鍵から導出した公開鍵が事前登録値と異なる場合は出力前に停止します。JSON receiptはRelease ID、manifestのartifact digest、tar全体のarchive digest、公開鍵、出力pathを返します。

Release候補のCIは次のように実行します。

```sh
AGENTIC_RELEASE_SIGNING_KEY_HEX=<64-character-ed25519-seed> \
AGENTIC_RELEASE_SIGNING_PUBLIC_KEY_HEX=<64-character-ed25519-public-key> \
AGENTIC_RELEASE_SOURCE_ID=remote:official \
AGENTIC_RELEASE_SIGNER_KEY_ID=framework.release.2026 \
sh scripts/release-ci.sh
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
distribution-trust.json
publish-receipt.json
SHA256SUMS
agentic-<target>[.exe]
agentic-<target>[.exe].build.json
publication-record.json
```

`publication-record.json`は候補run ID、source revision、取得元・署名鍵ID、Framework assetとbinary assetのdigestを記録する公開時の来歴情報です。Release署名やattestationの信頼元ではなく、どのCI runから公開したか追跡するための記録です。upload後検証に失敗した場合はdraftのまま停止し、自動削除や上書きを行いません。公開workflowもTrust Storeや導入先Framework lockは更新しません。

利用者はchecksumに加え、GitHub CLIでbinaryのprovenanceを検証できます。

```sh
gh attestation verify agentic-x86_64-unknown-linux-gnu \
  --repo <owner>/<repository> \
  --signer-workflow <owner>/<repository>/.github/workflows/vnext-release.yml
```

公開済みbinaryの初回導入は、Release tagを明示してbootstrapを実行します。POSIX環境では次のとおりです。

```sh
sh bootstrap/install.sh \
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

どちらもGitHub CLIを必要とします。Releaseのsource revisionと既定branchを取得し、Repository、候補生成workflow、source revision、source ref、GitHub-hosted runnerを固定して、実行binaryと`distribution-trust.json`のArtifact Attestationをそれぞれ検証してからbinaryを実行します。checksumやRelease内の公開鍵だけを信頼するmode、検証省略optionはありません。

検証済みbinaryは、導入先の`releases/<tag>/`へ不変のassetを保存し、`active`という2行の機械管理fileに現在tagと直前tagを記録します。`bin/agentic`または`bin/agentic.cmd`は現在tagのbinaryを起動します。CLI binaryの更新とProjectごとのFramework Release更新は別操作です。

```sh
agentic binary status --install-root /path/to/agentic

agentic binary update /path/to/downloaded-assets \
  --tag framework-<new-release-id> \
  --source-revision <40-character-git-sha> \
  --install-root /path/to/agentic

agentic binary rollback --install-root /path/to/agentic
```

bootstrapは公開assetの取得とattestation検証を含みます。`binary update`または`project init --candidate-dir`へdirectoryを直接渡す場合は、呼出し側がbinaryと`distribution-trust.json`のattestation検証を済ませる必要があります。binary manager自身はdirectoryのfile間整合性、Trust BundleとPublication Recordの一致を再検証しますが、GitHubへ接続してattestationを取得しません。

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
| `src/source_detection.rs` | 対応・inventory対象言語の登録、共通parse・正規化・分類・整列処理、言語横断conformanceを定義 |
| `src/framework_detection.rs` | 主要ORM・メッセージングAPIのproject/source根拠から、非authoritativeなframework method Binding候補を生成 |
| `src/python_detection.rs` | Python構文から関数・呼出先・物理resourceを機械的に観測 |
| `src/java_detection.rs` | Java構文からmethod・constructor・呼出先・物理resourceを観測 |
| `src/kotlin_detection.rs` | Kotlin構文からfunction・navigation call・物理resourceを観測 |
| `src/go_detection.rs` | Go構文からfunction・method・selector call・物理resourceを観測 |
| `src/rust_detection.rs` | Rust構文からfunction・field call・物理resourceを観測 |
| `src/ruby_detection.rs` | Ruby構文からmethod・singleton method・明示的receiver call・物理resourceを観測 |
| `src/php_detection.rs` | PHP構文からfunction・method・member/nullsafe/static call・物理resourceを観測 |
| `src/csharp_detection.rs` | C#構文からmethod・constructor・通常/null条件呼出し・物理resourceを観測 |
| `src/swift_detection.rs` | Swift構文からfunction・initializer・navigation call・物理resourceを観測 |
| `src/scala_detection.rs` | Scala 2/3構文からfunction・extension・通常/infix/postfix call・物理resourceを観測 |
| `src/c_detection.rs` | C構文からfunction・free function call・struct function pointer call・物理resourceを観測 |
| `src/gdscript_detection.rs` | Godot GDScript構文からscript class・inner class・function・property accessor・lambda・attribute call・Signal発火・物理resourceを観測 |
| `src/script_detection.rs` | JavaScript・JSX・TypeScript・TSX構文から同じ物理情報を観測 |
| `src/git_repository.rs` | Git解析対象の列挙、Binding Record適用、coverage生成 |
| `src/binding_validation.rs` | Binding違反とcoverageによる検査不能を分けた検証reportを生成 |
| `src/binding_draft_validation.rs` | review済みObservation Draftの完全性・authority・source freshnessを正式反映前に検証 |
| `src/detector_benchmark.rs` | review済みcorpusに対するDetector・framework候補のprecision／recallと閾値判定を生成 |
| `src/detector_audit_baseline.rs` | 固定Repositoryの全入力・監査Report・既知gapをreview済みbaselineと比較 |
| `src/detection.rs` | 正規化したrepository factからSignal候補を生成 |
| `src/signal_catalog.rs` | 標準Signal Domain、Signal、binding、typed repository factからの変換と、全Consumer共通の検証済みRegistryを提供 |
| `src/rules.rs` | Requirement・Ruleの構造検査とRule Index生成 |
| `src/kernel.rs` | Requirement選択、freshness、次状態を判定する純粋ロジック |
| `src/context.rs` | NextActionから実行用Contextと参照digestを生成 |
| `src/project_runtime.rs` | 実Projectのconfig、Release、Git観測、Storeを接続 |
| `src/migration.rs` | 現行CLI Projectを診断し、Migration Draftのレビュー検証、隔離候補の生成・整合性検証・明示適用を行う |
| `schemas/v1/` | 保存Recordの言語非依存Schema |
| `schemas/mcp/v1/` | Agent用MCP Toolの固定I/O Schema |
| `schemas/ci/v1/` | project所有のCI policy形式。Contract Healthの停止対象を明示する |
| `schemas/benchmarks/v1/` | Detector benchmark corpusとreview済み正解・閾値の固定形式 |
| `schemas/catalog/v1/` | 標準Signal Domain CatalogとFramework Detection Catalogの機械可読な固定形式 |
| `schemas/outputs/v1/` | CLI生成物の公開形式。Next Response、Explain Report、Binding Validation Report、Migration Application、Contract Health Report／Gate Report、Detector Benchmark／Repository Audit Report |
| `schemas/delivery/v1/` | 移行互換用の未署名Framework Release manifest |
| `schemas/delivery/v2/` | 署名済みRelease、attestation対象Distribution Trust、鍵statusを持つ公開鍵設定、取得元、Framework lock拡張、Publish Receipt、Binary Build Record、Publication Recordの固定形式 |
| `testdata/golden/v1/` | canonical JSON、Schema、Kernel、Application、永続lifecycle、Explain Report等の固定期待値 |
| `src/`、`tests/` | 正準実装のRust crate。Project loader、ProjectStore、Application、CLI、local stdio MCP serverを含む |
| `legacy/agentic_vnext/` | 過去の設計探索に使ったPython実装。新しい設計の実装対象にはしない |
| `testdata/fixtures/db-sqs/` | DB更新＋SQS送信の固定入力 |
| `testdata/fixtures/security-lifecycle/` | review済みSecurity BindingからEvidence・Challenge完了までを通す固定入力 |
| `testdata/benchmarks/major-frameworks-v1/` | 主要8 ORM・8 messaging・8 HTTP client・3 Object Storage SDK系統を扱う10 projectの品質corpus |
| `testdata/benchmarks/real-projects-v1/` | 固定revisionの4 OSS Repositoryから代表sourceを収録したoffline品質corpus |
| `testdata/benchmarks/repository-audits-v1/` | 固定revisionの4 OSS Repository全体に対するreview済み監査digestと既知gap |

## 現時点の制約

- InMemory Adapterはテスト用です。Filesystem StoreではChange、Contract、Decision、Result、Evidenceを保存しますが、発行済みAction自体は保存せず、正本から再生成します。
- 実Projectの通常経路では、Observation Schema v5に手書きの`facts`・`coverage`を置きません。Rust版が`analysis.roots`配下のGit上のsourceを言語登録表に従って列挙し、対応Detectorで解析して生成します。旧Schema v4の単数`logical_ref`・`kind`も読み込み互換として受理します。
- 現在の言語DetectorはPython、Java、Kotlin、Go、Rust、Ruby、PHP、C#、Swift、Scala、C、GDScript、JavaScript、JSX、TypeScript、TSXに対応します。C++はinventoryへ出しますが、構文Detectorは未実装なので宣言後も`unsupported-language`で停止します。
- `.js`・`.mjs`・`.cjs`もJSXを受理します。source拡張子はASCIIの大文字小文字を区別せず、Git inventoryでも同じ規則を使います。
- receiverは構文tokenを連結した1行の物理IDへ正規化します。たとえば複数行の`client .table("orders")`は`client.table("orders")`になります。同じ行に同一呼出しが複数あっても観測を重複除去しません。
- 組込み分類は言語ごとの自然な綴りだけを対象とします。C・GDScript・Python・Ruby・Rustは`send_message`、Java・Kotlin・PHP・Swift・Scala・JavaScript・TypeScript系は`sendMessage`、Go・C#は`SendMessage`です。GDScriptではGodot 4の`Signal.emit`と明示receiver付き`Object.emit_signal`もmessage publishです。DB書込みはGo・C#が`Insert`・`Update`・`Delete`、その他が小文字で始まる綴りです。`send`・`save`・`Save`など曖昧またはframework固有の名前は、`resource.method`ごとに`kind`、owner、承認DecisionをBindingします。
- Cのfree function callは第1引数を物理resource、関数名をmethodとして観測し、先頭のaddress-of `&`はresource名から除きます。`client->publish(...)`のようなstruct function pointer callはstruct式をresource、field名をmethodにします。引数のないcallはresource identityを持たないため観測しません。未Binding resource上の未分類callは候補から除外し、Binding済みresourceに対する`sqlite3_exec`や`PQexec`等は明示的なmethod Bindingがなければ停止します。
- GDScriptは`.gd`を対象とし、`class_name`とinner classをsymbolへ反映します。function、propertyの`set`・`get`、変数へ代入したlambdaに安定したsymbolを付け、`$Node`・`%UniqueNode`・`super`・chain receiverを物理resourceとして扱います。`signal_name.emit(...)`はsignal名をresourceとして観測します。receiverを省略した`emit_signal(...)`を含むbare callはresource identityを持たないため観測しません。
- Rustのturbofish付きmethod callとJavaScript・TypeScriptの文字列computed propertyを観測します。動的computed propertyも`OtherMethodCall`として残すため、Binding済みreceiverなら`unsupported-observation`で停止します。aliasと動的dispatchの意味解決は今後のDetector追加対象です。
- class・impl・receiver内のsymbolは型名で修飾します。既存の短縮Binding keyはartifact内で一意な場合だけ互換利用し、同名symbolが複数ある場合は`ambiguous-symbol-binding`で停止して修飾keyを要求します。TypeScriptのclass field関数、default export、CommonJS代入、Pythonの代入lambda・class body、Javaのstatic initializer、Swiftのinitializer・型property closure、Scalaの型初期化・extension receiver・型level val closureにも安定した物理symbolを割り当てます。
- Binding Recordはartifact内の関数名・物理resource名、および必要なframework固有methodごとに論理IDまたは観測kind、owner、承認Decisionを記録します。Schema v5では1つのresourceにbinding種別ごとの`logical_refs`、1つのmethodに複数の`fact_kinds`を記録でき、全組合せの妥当性を確認してから複数factを一括生成します。承認Decisionは`accepted`でなければならず、artifact・binding・承認Decisionの変更は検出根拠digestへ反映されます。
- `project observe`のDraft v6はsourceと生成元ObservationのSHA-256を固定し、主要8 ORM・8 messaging frameworkに加え、Requests、HTTPX、明示receiver付きFetch、Axios、Java HttpClient、Spring WebClient、Go `net/http`、.NET HttpClient、Amazon S3、Google Cloud Storage、Azure Blob Storageの候補を提示します。候補はBinding Recordへ自動反映せず、通常評価も参照しません。明確なObject Storage uploadは`external_call`と`object_write`の両方を候補にします。SQLAlchemy `execute`やJavaScript版S3 `client.send`など読書き両用APIは空の候補listとし、個別reviewを要求します。通常のbare `fetch()`はBinding可能なreceiver identityを持たないため候補化せず、`window.fetch`・`globalThis.fetch`・`self.fetch`だけを扱います。曖昧なmethod名は、対応するmanifest・import・型・receiverの根拠がある場合だけ候補化します。`binding_artifacts`は反映用構造だけを作り、fact kinds・論理ID・owner・承認先を`null`にするため、そのままではBinding Recordとして受理されません。`project promote-bindings`はDraftを再検証し、freshな場合だけ現在のphaseを保って正式Observationを原子的に置換します。
- 主要frameworkのE2Eは、Django/SQS、SQLAlchemy/Celery、Prisma/Kafka、Spring Data JPA/RabbitMQ、Entity Framework Core/Azure Service Bus、Rails/Redis Streams、Laravel/Google Cloud Pub/Sub、GORM/NATSの8 fixtureを、`observe`、review済みDraft検証、promotion、正式Binding検証、Signal生成まで同じCLI経路で通します。SQLAlchemy `execute`のように候補が空のAPIは、明示的な個別分類なしではE2Eを通しません。
- 外部連携のE2Eは、Requests、HTTPX、Fetch、Axios、Java HttpClient、Spring WebClient、Go `net/http`、.NET HttpClientの8系統と、Amazon S3（Python・JavaScript）、Google Cloud Storage、Azure Blob Storageの4実装を同じ経路で検証します。Object Storageは`external-system-call`と`object-storage-write`の両方を生成し、JavaScript版S3の`send`は明示reviewで両方に分類した場合だけ通します。
- Detector benchmarkは代表fixtureに加え、固定revisionのdjango-oscar、Prisma Examples、NATS Go、Godot Demo Projectsの選択sourceに対する品質回帰を測ります。`detector-audit`は外部clone全体のcoverageと候補分布を測り、review済みbaselineがrevision・全入力・Report digest・既知gapの変化を検出します。ただしRepository全件へ意味上の正解ラベルを付けたものではないため、全件の誤検知率とは呼びません。review済みsampleの拡大、動的dispatch、alias解析、実運用での誤検知率調査は今後のcorpus拡張対象です。
- Signal Domain Catalog v3は、既存の永続化・外部連携Signalに加え、review済みMethod Bindingから生成する`authorization_change`と`sensitive_data_access`を収録します。前者は`authorization-control-change`、後者は`sensitive-data-access`を出力し、それぞれ`authorization.*`と`data.*`のresource Bindingを要求します。Project固有の認可境界とdata分類をmethod名から推測せず、accepted Decisionによる明示Bindingだけを受理します。
- Framework Detection Catalog v1は、Framework Release開発者が組込み候補へruleを追加するための形式です。現在の公式Release候補にはTypeORMの永続化APIを収録しています。PublisherはCatalogを検証して署名対象assetへ追加し、`project observe`はactive lockが固定した署名済みReleaseからだけ読み込みます。外部IDは`namespace/name`へ正規化し、重複するframework・言語・methodの組合せを拒否します。導入Project独自のCatalogは受理せず、候補をBindingへ自動昇格しません。
- ContextはRequirement単位に分離していますが、現在の最小単位はContract文書IDとコードartifact IDです。Contract clauseやコードsymbol単位の選択は未実装です。
- Rust CLIはFramework lockからlocal Releaseを自動解決して`next`と`explain`を実行します。決定的な署名済みRelease生成、offline directory・local tar・remote tarの導入、切替、rollback、5 Platformのnative binary build、checksum・attestation、候補Artifact保存、Environment承認後のGitHub Release公開、attestation必須bootstrap、versioned binary更新・rollbackは実装済みです。実際のGitHub-hosted workflowによる公開・導入の実証、SBOM、認証付きFramework取得、resume、複数mirror、過去のResult IDを指定した説明は未実装です。
- Framework lock v2はRelease artifact digest、取得元ID、署名鍵IDを固定します。鍵のrotation・retire・revoke規則は実装済みです。組織提供Releaseの合成、署名済み失効listのremote同期、透明性logは未実装です。
- Result payload SchemaはPrototype組込みの6種類に固定されています。Project・組織固有Schemaの追加方法とSchema migrationは未実装です。
- lifecycle goldenは、実装でコードartifact digestが変わっても設計工程を繰り返さずEvidenceへ進む正常系を固定しています。stale Actionの拒否や保存競合等の異常系scenarioは通常テストだけで、まだ言語間golden化していません。
- `canonical-json-v1`は整数だけを扱います。浮動小数点の言語間正規化は未定義のため拒否します。
- Rust crateはcanonical JSONからExplain Reportまでgolden互換であり、実Projectのconfig、Git artifact、Filesystem Storeを読む`next`／`explain` CLIを含みます。
- cacheはwrite-throughのみです。検証済みcache readによる高速化は未実装です。
- cleanなlocal Git cloneでのCI再現と、明示policyによるRepository全体の定期Contract Healthゲートまで実装済みです。remote CI status、shallow clone、submodule、複数Repositoryは未実装です。
- build phaseと解析root、Binding RecordはProject manifestへ明示します。risk factとcoverageはRust AdapterがGitとsourceから生成します。
- MCP Adapterは発行済みActionに応じてResult、Evidence、Decision、Contractを書き込みます。Decision／Contract全体の更新は`expected_digest`、Contract条項の更新は`expected_clause_digests`による楽観的lockを使います。別条項の並行更新は保持し、同じ条項の並行更新はstaleとして拒否します。新規作成では`expected_digest: null`を明示します。remote MCP、複数Projectを扱う単一process、未提出Actionのprocess再起動を跨ぐresumeは未実装です。
- 現行`bin/agentic`、Schema、Skill、導入処理の挙動は変更しません。

したがって、このPrototypeのResult形式やModule APIを互換性のある公開仕様として利用しないでください。

## 配布実装との関係

広く配布するvNextでは、KernelとCLIをRustのbuild済みバイナリとして提供し、通常利用者にはPythonもRust toolchainも要求しない案を第一候補とします。現在は決定的な署名済みRelease生成、実Projectに対する`next`と`explain`、offline・remote Releaseの検証、atomic install、lock切替・rollback、5 Platformのnative binary、checksum・attestation、候補Artifact保存、承認付きGitHub Release公開まで接続済みです。Publisherの秘密鍵利用は候補生成CIに限定し、公開jobと通常利用者は公開鍵だけを保持します。詳細は`FRAMEWORK-REVIEW.md` 14.12、14.21、14.22を参照してください。
