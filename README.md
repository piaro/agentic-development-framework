# Agentic Development Kit

AIエージェントを前提に、Repository内の仕様・判断・実装・検証を接続する開発コントロールプレーンです。

次期構成の検討内容は`FRAMEWORK-REVIEW.md`、現行CLIの標準経路へ未接続の検証実装は`prototype/vnext/README.md`にあります。Prototypeは公開APIや現在の利用手順ではありません。vNextの実装・互換性検証はRust版を正とし、Python版は過去の設計検証用referenceとして残しています。Rustのbuild済みバイナリ、Artifact Attestation必須のbootstrap、versioned update・rollbackに加え、現行Projectを変更しない移行診断、Migration Draftのレビュー検証、隔離された候補生成と整合性検証までを配布候補として検証しています。

## 目的

Agentic Development Kitは、エージェントへ実装を依頼する前に「何が正しいか」「誰が決めたか」「何を証拠に完了とするか」を明示し、変更中に見つかった未決定事項を安全に人へ戻すための仕組みです。

主に次を実現します。

- 現在有効な仕様を階層Contractとして管理する
- Issue、Contract、Decisionに基づいて変更ごとの適用Contractを解決する
- Contract gapや仕様拡張を実装前Challengerが反証する
- authorityのないプロダクト判断を`blocked-contract-decision`で止める
- Data InvariantとOperation Contractからデータ変更経路を横断検査する
- BuilderとChallengerを分離し、証拠が揃った変更だけを完了とする
- 障害から得た知識をContract、test、probe、runtime checkへ昇格する

Challengerは不足・矛盾・反例を指摘できますが、新しいプロダクト仕様は決定しません。CLIもartifactの構造、参照、状態、hash、coverageを検査しますが、意味上の正しさは自動決定しません。

## 全体の動き

すべての変更は、概ね次のループを通ります。

```text
Issue・依頼・既存Docs
        │
        ▼
Change作成・影響範囲の復元                 $agentic-change
        │
        ▼
Contract Assessment・authority確認        $agentic-contract
        │
        ├─ 未決定 ─▶ Decision Request ─▶ 人の判断 ─┐
        │                                           │
        ◀───────────────────────────────────────────┘
        ▼
実装前Challenge                            $agentic-challenger
        │
        ▼
Contract resolve・readiness gate           CLI
        │
        ▼
実装                                       $agentic-builder
        │
        ▼
実装後Challenge・Evidence確認              $agentic-challenger / CLI
        │
        ▼
完了 ── 障害・再発知識 ─▶ Contract等へ昇格 $agentic-learning
```

基本のSkill実行順は、`$agentic-change` → `$agentic-contract` → 実装前`$agentic-challenger` → `$agentic-builder` → 実装後`$agentic-challenger`です。`$agentic-learning`は障害や横断的な学びが発生したときに実行します。

R1以上、またはAssessmentにdecisionがある変更では実装前Challengeが必須です。R2 / R3では、実装前・実装後ともBuilderから独立したcontextのChallengerを使います。decisionのないR0では、実装前Challengeを省略できます。

各段階は次のartifactを受け渡します。

| 段階 | 主な入力 | 主な出力・gate |
|---|---|---|
| Change | Issue、依頼、既存Docs、コード | `change.yaml`、影響範囲、Risk |
| Contract | Change、accepted Contract、Decision | `contract-assessment.yaml`、Feature Contract、Decision Request |
| 実装前Challenge | Issue、Assessment、authority、Contract | `contract-challenge.yaml`、blocking finding |
| Resolve / Ready | Assessment、Challenge、Contract、active change | `.agentic/resolved/<id>.lock.yaml`、readiness |
| Build | freshなresolved lock | コード、テスト、migration、証拠 |
| 実装後Challenge | raw diff、Mutation Graph、Operation Contract | 反証結果、残存リスク、追加証拠 |
| Evidence | Contract clause、テスト、probe | 完了可否 |

Assessment、Decision Request、Challenge、resolved lockは変更単位のworkflow情報です。現在有効な規範の正本は`contracts/`、判断履歴の正本は`decisions/`に置きます。

## Quick Start

### 前提環境

- `bin/agentic-init`: POSIX互換`sh`
- `.agentic/bin/agentic`: Python 3.10以上、PyYAML 6系
- 対象Repositoryの仕様、Issue、コード、テストを調査できるエージェント環境

導入後にPython依存をインストールします。

```sh
python3 -m pip install -r .agentic/runtime/requirements.txt
```

### 導入Levelを選ぶ

上位Levelは下位Levelを含みます。現在のコード量ではなく、失敗時の影響、状態の複雑さ、変更の不可逆性で選びます。

| Level | 主な対象 | 導入される仕組み |
|---|---|---|
| Lite | 個人開発、短期プロトタイプ | Project / Feature Contract、authority、Decision Request、実装前Challenge |
| Standard | 継続運用する通常の業務アプリ | Lite + Domain / Capability / Architecture、Data Invariant、Operation Contract、適合性検査 |
| System | 複数サービス・チーム・エージェント、非同期処理 | Standard + Mutation Graph、active change競合、Platform probe、Stateful Challenger |
| Critical | 金銭、機密、権限、法令、監査、不可逆操作 | System + Threat / Failure Model、Failure Injection、runtime invariant、release evidence |

迷った場合はStandardを基準にします。プロジェクトの導入Levelとは別に、各変更をR0からR3で分類します。保存形式、認可境界、外部Protocol、削除などの高リスク変更では、プロジェクト全体のLevelを変えずに必要な検証だけを一時昇格できます。

### 対話形式で導入する

KitのRepositoryで次を実行し、導入モード、対象、Levelを選択します。

```sh
./bin/agentic-init
```

`new`は空のディレクトリへ新規導入し、`adopt`は既存Repositoryへ導入します。

### 新規プロジェクトへ導入する

```sh
./bin/agentic-init \
  --mode new \
  --level standard \
  --target /path/to/new-project \
  --name example-project \
  --non-interactive
```

### 既存プロジェクトへ導入する

まず`--dry-run`で追加予定を確認します。既存のユーザーファイルは上書きしません。

```sh
./bin/agentic-init \
  --mode adopt \
  --level standard \
  --target /path/to/existing-project \
  --non-interactive \
  --dry-run

./bin/agentic-init \
  --mode adopt \
  --level standard \
  --target /path/to/existing-project \
  --non-interactive
```

導入直後は、`docs/agentic/adoption-report.md`と`docs/agentic/source-of-truth.md`をエージェントに完成させ、コードから推測した事実と認可済みの仕様を区別します。

### 最初の変更を開始する

対象Repositoryで次を実行します。

```sh
.agentic/bin/agentic contract lint
.agentic/bin/agentic change init feature-id --title "変更タイトル"
```

生成された`.agentic/changes/feature-id/`と`contracts/features/feature-id.yaml`を、後述のDevelopment flowに沿って更新します。

## 動作の前提となるDocsと情報源

Kitは、エージェントの推論だけを仕様の根拠にはしません。変更開始前に、少なくとも次の情報へアクセスできる状態にします。

| 情報 | 役割 | 仕様を決めるauthorityになれるか |
|---|---|---|
| `AGENTS.md` | Repository固有の作業規約、禁止事項、検証方法 | Assessment decisionのauthority kindにはしない |
| Issue・要求文 | 変更の目的、明示要求、非対象、受入条件 | 明示された要求は`issue-requirement`として可 |
| `contracts/` | 現在有効な規範 | acceptedな明示clauseは`accepted-contract`として可 |
| `decisions/` | 判断理由と変更履歴 | accepted Decisionは`accepted-decision`として可 |
| 記録された人の判断 | 選択された仕様と判断者 | `human-decision`として可 |
| `docs/agentic/source-of-truth.md` | 既存文書とContractの対応、正本の所在 | 索引。参照先のauthorityを置き換えない |
| コード・テスト | 現在の実装事実、回帰証拠 | 単独では不可 |
| `evidence/`・`probes/` | Platform能力と検証結果 | 事実の証拠。単独では新仕様のauthorityにしない |

Agent推論、Challenger finding、Contract gap、実装都合、既存コードだけ、テストだけでは、新しいプロダクト仕様を決定できません。

導入時に生成されるDocsは次の役割に限定します。

| Docs | 内容 |
|---|---|
| `docs/agentic/README.md` | 導入先Repositoryでの運用入口 |
| `docs/agentic/source-of-truth.md` | 現在の正本と既存文書の対応表 |
| `docs/agentic/adoption-report.md` | 既存実装の調査結果、差異、移行候補、未確認事項 |

`docs/`は利用案内と索引です。現在有効な規範を`docs/`だけに閉じ込めず、`contracts/`へ昇格します。

## Development flow

### 基本フロー: 通常の機能変更

1. `$agentic-change`がIssueとRepositoryを調査し、成果、非対象、影響するContext、Entity、Capability、Operation、Interface、Path、Riskを記録する。
2. `agentic contract candidates <id>`が単一軸一致するContract候補を列挙する。
3. `$agentic-contract`が候補を意味的に評価し、適用、除外理由、decision、authority、gap、Platform未知をAssessmentへ記録する。
4. Feature Contractへ今回の成果、失敗・完了の意味、上位Contractとの差分、証拠要件を記録する。resolve対象となるFeatureと上位Contractは、必要なauthorityと合意を得て`accepted`にする。
5. `$agentic-challenger`が実装前にIssue、authority、decision、Featureと上位Contractを独立に反証する。
6. `agentic contract resolve <id>`でaccepted Contract、authority、Challenge、除外判断をhash付きlockへ固定する。
7. `agentic change ready <id>`がlockの鮮度、未解決事項、Contract coverage、Mutation競合を確認する。
8. `$agentic-builder`がresolved lockに従って実装する。新しい仕様判断が見つかった場合は実装を続けずAssessmentへ戻す。
9. `$agentic-challenger`が実装後にraw diff、Mutation Graph、Operation Contract、テスト、probeを使って反証する。
10. `agentic evidence check <id>`でContract clauseに対応する証拠と残存リスクを確認する。

候補一致は意味上の適用を自動確定しません。Project、Featureの明示参照、Operation/APIの厳密一致は必須契約とし、それ以外の候補はエージェントが内容を評価します。

### ユースケース: 仕様判断が足りない

既存authorityから一意に決められない場合、Assessment decisionを`needs-human-decision`にし、問い、選択肢、影響、推奨、必要な判断者をDecision Requestへまとめます。この時点のchangeは`blocked-contract-decision`です。

```sh
.agentic/bin/agentic contract decisions <change-id>
.agentic/bin/agentic contract decisions --all --format markdown
```

人の判断後は、判断履歴を`decisions/`へ、現在値を持つ判断なら有効な仕様を`contracts/`へ反映し、Assessmentのauthorityから参照します。Decision Requestは一時情報であり、解決後の実装やContractから参照し続けません。

### ユースケース: 新しい仕様や上位Contract変更が必要

新しいEntity、API、必須入力、権限、ownership、cardinality、lifecycle、retention、Protocol、error、idempotency、外部作用などは仕様拡張として扱います。

1. 既存accepted Contract、Issue明示要求、accepted Decision、記録された人の判断に根拠があるか確認する。
2. 根拠がなければDecision Requestを作り、人へ判断を求める。
3. Feature固有でない判断は、適切なProject / Domain / Capability / Architecture / Data Invariant / Operation Contractへ反映する。
4. resolvedな仕様拡張decision IDをFeature Contractの`introduced_decisions`へ対応づける。
5. 実装前Challengeをやり直し、authorityがdecisionの内容を本当に支持するか反証する。

Challengerのfindingは再検討の入口にはなりますが、authorityにはなりません。

### ユースケース: データを変更する

Standard以上では、操作順序に依存せず守る条件をData Invariantへ、各writeの振る舞いをOperation Contractへ記録します。その後`agentic mutation build`で同じEntityへのwriterとactive change競合を可視化します。

System以上またはR2以上の変更では、create / update / delete / retry、並行実行、順序逆転、commit前後の停止、event重複、migration混在をScenario Driverで試し、各操作後にInvariantを検査します。

### ユースケース: 既存Repositoryへ導入する

既存コードをそのまま正しい仕様とみなさず、次の順で採用します。

1. 実行入口、境界、データストア、外部サービス、CI、テスト、既存仕様を収集する。
2. Domain、状態、所有権、多重度、変更経路、Platform未知を復元する。
3. 同じ能力の異なる実装を比較し、`adoption-report.md`へ事実、推測、未確認を分けて記録する。
4. 認可済みの現在値だけをContractへ昇格し、既存文書との対応を`source-of-truth.md`へ記録する。
5. 安全網、参照設計、互換層、data migration、呼び出し側、旧経路の順で段階移行する。

### ユースケース: 障害や反復する不具合から学ぶ

`$agentic-learning`が、発生条件、破られたInvariant、見逃した境界、検知できなかった理由を分析します。再発防止の知識は、影響範囲に応じて上位Contract、Operation Contract、共通test、Platform probe、runtime checkerへ昇格します。

incident findingだけで新しい仕様を決めることはせず、プロダクト判断が必要ならDecision Requestへ戻します。

## Contractの概念とヒエラルキー

Contractは「現在このRepositoryで守るべき正しさ」を、エージェントとCLIが参照できる構造で表したものです。設計資料の要約ではなく、変更のreadinessと完了判定に使う規範です。

```text
Project Contract                         Repository全体の原則
  ├─ Domain Contract                    用語、Entity、関係、lifecycle
  ├─ Capability Contract                業務能力の入出力、完了、互換性
  ├─ Architecture Contract              責務、依存方向、参照実装
  ├─ Data Invariant                     操作をまたいで常に守る状態
  └─ Operation Contract                 個々のread/writeと失敗・再試行

Feature Contract                         今回の変更差分
  └─ governing_contracts ──────────────▶ 適用する上位Contract群
```

Feature Contractは上位Contractのコピーでも上書きでもありません。今回の成果と差分を表し、適用する上位Contractを`governing_contracts`で参照します。所有権、多重度、状態遷移、共通Protocol、保存形式などFeature外でも有効な判断は、上位Contractで決めます。

### ContractとDecisionの違い

| Artifact | 答える問い | 保持期間 |
|---|---|---|
| Contract | 今、何を守るか | 現在有効な間 |
| Decision | なぜ、その仕様を選んだか | 判断履歴として保持 |
| Assessment | この変更に何が適用されるか | 変更workflow中と履歴 |
| Decision Request | 人に何を決めてほしいか | 解決までの一時情報 |
| Challenge | どの前提を反証し、何が残ったか | 変更workflowの証拠 |
| resolved lock | 実装が従うContract集合は何か | 対象入力が変わるまで |

### Authorityとreadiness

Assessmentで`resolved`または`accepted`にするdecisionには、次のいずれかのauthorityが必要です。

- `accepted-contract`: 既存accepted Contractの明示clause
- `issue-requirement`: Issue本文の明示要求
- `human-decision`: 記録された人の判断
- `accepted-decision`: acceptedなDecision record

CLIはauthority kind、ref、参照先の状態、Featureとの対応を構造検査します。Issueや人の判断がdecisionの意味を本当に支持するかは、実装前Challengerが独立に検査します。

未判断候補、authority不足、未解決Decision Request、Contract coverage不足、staleまたはblockingなChallenge、Platform未知、Contract競合があればresolve / readyは失敗します。Contract、Assessment、Repository内のauthority source、Challengeの内容が変わった場合も、既存lockはstaleになり再Challenge・再resolveが必要です。外部Issueなどの内容変更はofflineのhash検査では検知できないため、Challengerが参照内容を再確認します。

## Data Integrity

Data Integrityは、個別APIのテストだけでなく、同じデータへ到達するすべての操作とその組み合わせを対象にします。

### Data Invariant

操作の種類や順序に関係なく、観測可能な状態が満たす条件です。

例:

- 親を持たない子recordが存在しない
- 同じ業務識別子にactive recordが複数存在しない
- 完了済み操作の再試行で永続状態や外部作用が重複しない
- tenant境界を越えて参照できない

### Operation Contract

各操作について、Invariantをどう守るかを定義します。

- preconditionとreads
- mutation対象とaction
- transaction / atomic group
- external effect
- postconditionとconsistency
- idempotencyとduplicate semantics
- failure pointと競合Operation

### Mutation GraphとStateful Challenge

```sh
.agentic/bin/agentic mutation build
```

Mutation GraphはOperation ContractからEntityごとのwriterを集約し、単体では正しくても組み合わせでInvariantを破る経路や、同じ領域を変更するactive changeを見つけるために使います。

Stateful Challengerは、通常順だけでなく、重複、逆順、並行、timeout、部分失敗、cancel、migration中の新旧混在を試します。新しい意味を推測で追加するのではなく、反例が示すContract gapをAssessmentへ戻します。

## CLI

導入先では`.agentic/bin/agentic`を使用します。別のディレクトリから実行する場合は`--root /path/to/project`を指定できます。

| コマンド | 役割 |
|---|---|
| `agentic contract lint` | Contractの必須field、ID、参照、statusを構造検査する |
| `agentic change init <id> --title <title>` | Change、Assessment、Challenge、Feature Contractの雛形を生成する |
| `agentic contract candidates <id>` | 影響範囲と単一軸一致するContract候補を列挙する |
| `agentic contract decisions <id>` | 未解決Decision Requestを表示する |
| `agentic contract decisions --all --format markdown` | active changeの判断依頼を人向けにまとめる |
| `agentic contract challenge-input <id>` | 実装前Challengeへ渡す対象とfresh hashを出力する |
| `agentic contract authority-check <id>` | authority model、Feature対応、Challenge、lockを診断する |
| `agentic contract authority-check --all` | すべてのactive changeを診断する |
| `agentic contract resolve <id>` | 適用Contractとauthority検証結果をresolved lockへ固定する |
| `agentic mutation build` | Operation ContractからMutation Graphを構築する |
| `agentic change ready <id>` | 実装開始前のreadinessとactive change競合を検査する |
| `agentic evidence check <id>` | Featureのevidence requirements、Challenger結果、残存リスクを検査する |

代表的な実行順です。

```sh
.agentic/bin/agentic change init feature-id --title "変更タイトル"
.agentic/bin/agentic contract candidates feature-id
.agentic/bin/agentic contract lint
.agentic/bin/agentic contract challenge-input feature-id
# $agentic-challengerがcontract-challenge.yamlを完成させる
.agentic/bin/agentic contract resolve feature-id
.agentic/bin/agentic mutation build
.agentic/bin/agentic change ready feature-id
# 実装と実装後Challenge
.agentic/bin/agentic evidence check feature-id
```

CLIは意味判断を自動化しません。候補の採否、authorityが要求を意味的に支持するか、Feature-localか上位Contractか、どの選択肢がプロダクトとして正しいかは、Skillが情報を整理し、必要に応じて人が決定します。

## Repository内の情報配置

| 場所 | 役割 |
|---|---|
| `contracts/` | 現在有効な規範（What） |
| `decisions/` | 規範を決めた理由と変更履歴（Why） |
| `docs/agentic/` | 利用方法、既存正本との対応、導入報告（How / Index） |
| `probes/` | 外部Platformを実証する実行物 |
| `evidence/` | Contract clauseに対応するテスト、probe、反証、残存リスク |
| `.agentic/changes/<id>/` | Change、Assessment、Decision Request、Challenge、resolved lock |
| `.agentic/active-changes.yaml` | 進行中変更と依存・競合関係 |
| `.agentic/generated/mutation-graph.yaml` | Entity writerと変更競合の生成結果 |

## Kitのupgrade

導入済みRepositoryを更新する場合は、最新版のKitから実行します。

```sh
./bin/agentic-init --upgrade --target /path/to/project --non-interactive --dry-run
./bin/agentic-init --upgrade --target /path/to/project --non-interactive
/path/to/project/.agentic/bin/agentic --version
```

`--upgrade`は`.agentic/installation.yaml`からmode、Level、project名を引き継ぎ、Kit管理のCLI、Skill、`AGENTS.md`管理ブロック、installation metadataだけを更新します。既存のContract、Assessment、Decision、Schema、設定、Docsは上書きしません。

### 2.xから3.0.0への移行

3.0.0ではdecision authorityと実装前Challengeを必須化しました。

1. `--upgrade --dry-run`で更新対象を確認してから`--upgrade`する。
2. `agentic contract authority-check --all`でactive changeを診断する。
3. activeなAssessmentをschema version 2へ更新し、resolved / accepted decisionへauthorityを追加する。意味を判断できない項目は自動backfillせずDecision Requestにする。
4. resolvedな仕様拡張decision IDをFeature Contractの`introduced_decisions`へ追加する。
5. `agentic contract challenge-input <id>`を基に実装前Challengeをやり直す。
6. Challenge通過後に`contract resolve`と`change ready`を再実行する。

旧resolved lockは3.0.0のreadinessには使用できません。completed / cancelled changeのAssessmentとresolved lockは移行対象外です。一方、accepted Feature Contractは完了済みchangeに由来していても`contract lint`の対象です。旧形式の`introduced_decisions`がobject配列なら、既存のdecision IDだけを保持した文字列配列へ手動で変換します。

## Shell・CLI・エージェント・人の責務

| 担当 | 責務 |
|---|---|
| `agentic-init` | Level別template、CLI、Skill、AGENTS管理ブロックを安全に配置する |
| CLI | 構造、参照、status、hash、coverage、競合、証拠不足を機械検査する |
| Agent Skills | Repository調査、Contract記入、選択肢整理、実装、意味的な反証を行う |
| 人 | authorityのないプロダクト判断、risk受容、組織的な優先順位を決定する |

人へすべてのgapを丸投げするのではなく、エージェントは既存authorityから解決できるものを処理し、未決定の問いだけを選択肢、影響、推奨とともに提示します。

## Kit開発時のValidation

Kit自体を変更した場合は次を実行します。

```sh
sh -n bin/agentic-init
python3 -m py_compile bin/agentic
sh tests/test-init.sh
python3 /path/to/skill-creator/scripts/quick_validate.py skill-src/<skill-name>
```
