# Agentic Development Kit

AIエージェントを前提に、Repository内の仕様・判断・実装・検証を接続する開発コントロールプレーンです。

実装の詳細は`docs/implementation.md`、設計の検討記録は`docs/FRAMEWORK-REVIEW.md`にあります。約束する範囲と予告なく変わる範囲は`COMPATIBILITY.md`にあります。

## 目的

Agentic Development Kitは、エージェントへ実装を依頼する前に「何が正しいか」「誰が決めたか」「何を証拠に完了とするか」を明示し、変更中に見つかった未決定事項を安全に人へ戻すための仕組みです。

主に次を実現します。

- 現在有効な仕様を階層Contractとして管理する
- Issue、Contract、Decisionに基づいて変更ごとの適用Contractを解決する
- Contract gapや仕様拡張を実装前Challengerが反証する
- 権限ある根拠のないプロダクト判断を止め、選択肢とともに人へ戻す
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
   agentic change init
        │
        ▼
┌─▶ agentic next ─── 次にやること1件を返す
│       │
│       ├─ Analyst   ─▶ 検出候補の確認、影響範囲と操作境界の確定、Contract記入
│       │                 └─ 未決定 ─▶ 人の判断 ─▶ Decision・Contractへ記録
│       ├─ Builder   ─▶ 実装、Contract条項に対応する証拠の記録
│       └─ Challenger ─▶ 実装前・実装後の反証（独立した文脈）
│       │
└───────┴─ agentic submit ─── 結果を検証・保存し、再評価する
        │
        ▼
     完了判定
```

エージェントがSkillの実行順を覚えるのではなく、`agentic next`が変更の状態から次の1件を決めます。エージェントはそれを実行して結果を提出し、また次を受け取ります。

役割ごとに使うSkillは3つです。

| 役割 | Skill | 担当する作業 |
|---|---|---|
| Analyst | `$agentic-analyst` | 検出候補の確認、影響範囲と操作境界の確定、Contract記入、人への判断依頼、回答の記録 |
| Builder | `$agentic-builder` | 実装と証拠の記録 |
| Challenger | `$agentic-challenger` | 実装前と実装後の反証 |

実装後の反証は、実装した文脈から独立した文脈で行います。同じ文脈での見直しを反証として記録しません。

発行される作業と、それに対して提出する結果は次のとおりです。

| 状態 | 割り当てられる作業 | 提出する結果 |
|---|---|---|
| `needs-analysis` | 検出候補の確認、要件の分析 | 候補の採否と理由、各要件の判定と根拠 |
| `needs-human-decision` | 人への判断依頼 | 人が選んだ選択肢と決定者 |
| `needs-decision-recording` | 回答をDecisionとContractへ反映 | 反映の完了 |
| `needs-pre-build-challenge` | 実装前の反証 | 各要件の判定と、攻めた内容 |
| `ready-to-build` | 実装 | 実装の要約 |
| `needs-evidence` | 証拠の記録 | Contract条項に対応する証拠 |
| `needs-post-build-challenge` | 実装後の反証 | 各要件の判定と、見つけた反例 |
| `ready-to-merge` | なし | — |

変更ごとの記録は`.agentic/changes/<id>/`に残ります。現在有効な規範の正本は`contracts/`、判断履歴の正本は`decisions/`です。

## Quick Start

### 前提環境

- 署名済みの`agentic`バイナリ。導入手順は`docs/implementation.md`にあります
- Git管理下のRepository。初期化はGitのtop-levelを対象にします
- 対象Repositoryの仕様、Issue、コード、テストを調査できるエージェント環境

利用者にPythonもRustのビルド環境も要求しません。

### プロジェクトを初期化する

```sh
agentic project init --project /path/to/project
```

`project init`は既存ファイルを上書きしません。次を配置します。

- `.agentic/` に設定、Framework lock、Trust Store、Release cache、空のRepository Observation
- `.agents/skills/` に3つのSkillと参照文書
- `docs/agentic/README.md` に進め方の案内
- `AGENTS.md` に管理ブロック。既にファイルがあれば末尾へ追記します

生成されたファイルは自動でcommitしません。内容を確認してからGitへ追加してください。

### 既にコードがあるとき

対応言語の物理的な関数とresourceを列挙し、レビューしてから正式な対応付けにします。候補は自動では反映されません。

```sh
agentic project observe \
  --project /path/to/project \
  --output .agentic/repository-observation.draft.yaml

# 人が論理ID、owner、承認Decisionを記入してから
agentic project validate-bindings --draft .agentic/repository-observation.draft.yaml --project /path/to/project
agentic project promote-bindings --draft .agentic/repository-observation.draft.yaml --project /path/to/project
```

### 最初の変更を開始する

```sh
agentic change init change.first-feature \
  --title "最初の機能" \
  --intent "何のために変更するか" \
  --project /path/to/project

agentic next change.first-feature --project /path/to/project
```

以降は`next`が返す作業を1件ずつ実行します。詳しい流れは次章にあります。

### 既存プロジェクトからの移行

旧CLIを導入済みのRepositoryは、`agentic migration inspect`で移行対象を診断できます。この機能は実験的な扱いで、初版の互換性の約束には含めません。手順は`docs/implementation.md`にあります。

## 動作の前提となるDocsと情報源

Kitは、エージェントの推論だけを仕様の根拠にはしません。変更開始前に、少なくとも次の情報へアクセスできる状態にします。

| 情報 | 役割 | 仕様を決めるauthorityになれるか |
|---|---|---|
| `AGENTS.md` | Repository固有の作業規約、禁止事項、検証方法 | 仕様を決める根拠にはしない |
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

1. `agentic change init <id>`で変更を作る。
2. `agentic next <id>`が次にやること1件を返す。以降はこれを繰り返す。
3. Analystが、検出された候補を実際のコードと突き合わせて採用または除外し、影響するデータと操作境界を確定する。必要な規範が無ければContractへ記入する。
4. 既存の権限ある根拠で決められない判断が出たら、選択肢、影響、推奨、必要な決定者を添えて人へ戻す。人が答えたら、理由をDecisionへ、現在の規範をContractへ記録する。
5. Challengerが実装前に、依頼、権限、判断、提案されたContractを反証する。
6. 実装前に必要な項目がすべて満たされると、Builderへ実装が割り当てられる。
7. Builderが実装し、Contract条項に対応する証拠を記録する。実装中に新しい仕様判断が出たら、実装を止めて分析へ戻す。
8. Challengerが実装後に、変更差分、データ不変条件、テスト、証拠を使って独立に反証する。
9. すべて満たされると完了できる状態になる。

判定の理由は`agentic explain <id>`で確認できます。

検出された候補の一致は、意味上の適用を自動確定しません。名前が似ているという理由で採用せず、実際のコードを読んで判断します。

### ユースケース: 仕様判断が足りない

既存の権限ある根拠から一意に決められない場合、問い、選択肢、影響、推奨、必要な判断者を判断依頼としてまとめます。変更は`needs-human-decision`で止まり、人が答えるまで先へ進みません。

```sh
agentic next <change-id>
agentic explain <change-id>
```

人の判断後は、判断の理由を`decisions/`へ、そこから決まった現在の規範を`contracts/`へ記録します。判断依頼は一時的な情報であり、以降の実装やContractから参照し続けません。

### ユースケース: 新しい仕様や上位Contract変更が必要

新しいEntity、API、必須入力、権限、ownership、cardinality、lifecycle、retention、Protocol、error、idempotency、外部作用などは仕様拡張として扱います。

1. 既存accepted Contract、Issue明示要求、accepted Decision、記録された人の判断に根拠があるか確認する。
2. 根拠がなければDecision Requestを作り、人へ判断を求める。
3. Feature固有でない判断は、適切なProject / Domain / Capability / Architecture / Data Invariant / Operation Contractへ反映する。
4. 決まった内容を、理由はDecisionへ、現在の規範はContractへ記録する。
5. 実装前の反証をやり直し、その根拠が本当にその判断を支持するか確かめる。

Challengerのfindingは再検討の入口にはなりますが、authorityにはなりません。

### ユースケース: データを変更する

操作順序に依存せず守る条件をData Invariantへ、各writeの振る舞いをOperation Contractへ記録します。書込みの検出は`project observe`が出した候補を人がレビューした対応付けに基づきます。

影響の大きい変更では、作成・更新・削除・再試行、並行実行、順序逆転、commit前後の停止、event重複、migration混在といった順序を試し、各操作後にInvariantを検査します。反証の観点は`agentic-challenger`のSkillにある参照文書にまとめてあります。

### ユースケース: 既存Repositoryへ導入する

既存コードをそのまま正しい仕様とみなさず、次の順で採用します。

1. 実行入口、境界、データストア、外部サービス、CI、テスト、既存仕様を収集する。
2. Domain、状態、所有権、多重度、変更経路、Platform未知を復元する。
3. 同じ能力の異なる実装を比較し、事実、推測、未確認を分けて記録する。
4. 認可済みの現在値だけをContractへ昇格し、既存文書との対応を残す。
5. 安全網、参照設計、互換層、data migration、呼び出し側、旧経路の順で段階移行する。

### ユースケース: 障害や反復する不具合から学ぶ

発生条件、破られたInvariant、見逃した境界、検知できなかった理由を分析します。再発防止の知識は、影響範囲に応じて上位Contract、Operation Contract、共通test、Platform probe、runtime checkerへ昇格します。この分析は専用のSkillではなく、通常の変更として扱います。

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
| 判断依頼 | 人に何を決めてほしいか | 解決までの一時情報 |
| 反証の結果 | どの前提を攻め、何が残ったか | 変更ごとの証拠 |
| 証拠 | Contract条項が本当に満たされたか | 変更ごとの証拠 |

### Authorityとreadiness

要件を満たしたと報告するには、次のいずれかの根拠が必要です。

- 既存のaccepted Contractの明示clause
- 依頼に明示された要求
- 記録された人の判断
- acceptedなDecision record

制御基盤は根拠の種類、参照先、その状態を構造として検査します。その根拠が判断の内容を本当に支持するかは、実装前の反証が独立に確かめます。

未確認の候補、根拠の不足、未解決の判断依頼、Contract coverageの不足、Platformの未知、Contractの競合があれば、実装へ進みません。Contract、コード、根拠となる記録が変わると、それに依存していた結果は古いものとして扱われ、やり直しになります。外部Issueなど、Repositoryの外にある内容の変更はhash検査では検知できないため、反証側が参照内容を再確認します。

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

### 状態を持つ変更への反証

同じEntityへ書き込む経路を並べ、単体では正しくても組み合わせでInvariantを破る順序を探します。通常順だけでなく、重複、逆順、並行、timeout、部分失敗、cancel、移行中の新旧混在を試します。

反証で新しい意味を推測で追加することはしません。反例が示すContractの不足は、分析へ戻して判断依頼にします。観点は`agentic-challenger`のSkillにある参照文書にまとめてあります。

## CLI

対象Repositoryは`--project /path/to/project`で指定します。省略した場合は現在のディレクトリです。

| コマンド | 役割 |
|---|---|
| `agentic project init` | 設定、Framework lock、Skill、案内、`AGENTS.md`の管理ブロックを配置する |
| `agentic project observe` | コードから物理的な関数とresourceを列挙し、レビュー用のDraftを出力する |
| `agentic project validate-bindings` | 対応付けの不足と検査不能な範囲を分けて報告する |
| `agentic project promote-bindings` | レビュー済みDraftを正式な観測結果へ反映する |
| `agentic change init <id> --title <title> --intent <intent>` | 変更を作る |
| `agentic next <id>` | 現在の状態と、次にやること1件を返す |
| `agentic explain <id>` | その判定になった理由を説明する |
| `agentic contract-health` | Repository全体のContractの健全性を検査する |
| `agentic mcp` | エージェント向けにMCP serverとして起動する |
| `agentic migration <...>` | 旧CLIのProjectを診断し、移行候補を作って適用する |
| `agentic release <...>` | 署名済みFramework Releaseを生成、取得、導入、切替、切り戻しする |
| `agentic binary <...>` | CLIバイナリ自体を導入、更新、状態確認、切り戻しする |

エージェントの通常経路はMCPです。`agentic mcp`を起動すると、`agentic_next`と`agentic_submit`で同じやり取りを行えます。

CLIは意味判断を自動化しません。候補の採否、根拠が要求を本当に支持するか、Feature固有か上位の規範か、どの選択肢がプロダクトとして正しいかは、Skillが情報を整理し、必要に応じて人が決定します。

## Repository内の情報配置

| 場所 | 役割 |
|---|---|
| `contracts/` | 現在有効な規範（What） |
| `decisions/` | 規範を決めた理由と変更履歴（Why） |
| `docs/agentic/` | 利用方法、既存正本との対応、導入報告（How / Index） |
| `probes/` | 外部Platformを実証する実行物 |
| `evidence/` | Contract clauseに対応するテスト、probe、反証、残存リスク |
| `.agentic/changes/<id>/` | 変更ごとの記録、発行された作業の結果、判断依頼 |
| `.agentic/config.yaml` | 正本の場所と観測結果の位置 |
| `.agentic/framework.lock` | 使用するFramework Releaseの固定 |
| `.agentic/repository-observation.yaml` | コード上の物理識別子と論理IDの対応付け |
| `.agents/skills/` | エージェント向けSkill。`project init`が配置する |

## 更新と切り戻し

CLIバイナリの更新と、Projectごとに使うFramework Releaseの更新は別の操作です。

```sh
agentic binary status
agentic binary update <candidate-directory>
agentic binary rollback

agentic release switch <release-id> --project /path/to/project
agentic release rollback --project /path/to/project
```

いずれも署名とattestationの検証を通ったものだけを導入します。詳細は`docs/implementation.md`にあります。

## Shell・CLI・エージェント・人の責務

| 担当 | 責務 |
|---|---|
| 制御基盤 | 次にやることを決め、構造、参照、状態、hash、coverage、競合、証拠不足を機械検査する |
| Agent Skills | Repository調査、Contract記入、選択肢整理、実装、意味的な反証を行う |
| 人 | 権限ある根拠のないプロダクト判断、risk受容、組織的な優先順位を決定する |

人へすべてのgapを丸投げするのではなく、エージェントは既存authorityから解決できるものを処理し、未決定の問いだけを選択肢、影響、推奨とともに提示します。

## Repositoryの構成

Rust実装がRepository直下にあります。

| 場所 | 内容 |
|---|---|
| `Cargo.toml`、`src/`、`tests/` | 正準実装のRust crate。バイナリ名は`agentic` |
| `schemas/` | 保存Recordと生成物の言語非依存Schema。Framework Releaseにも含まれる |
| `skill-src/`、`templates/` | バイナリへ同梱し、`project init`が配置するSkillと案内 |
| `scripts/` | リリース生成・検証・公開の補助script。`scripts/tests/`に一連の受入テスト |
| `bootstrap/` | 配布バイナリの導入script |
| `testdata/` | golden期待値、固定入力、Detector品質corpus |
| `docs/` | 実装の正本`implementation.md`と設計記録 |

## Kit開発時のValidation

Kit自体を変更した場合は次を実行します。

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
sh scripts/tests/test-vnext-rust.sh
```

`scripts/tests/test-vnext-rust.sh`は、書式、lint、テストに加えて、Detector品質corpus、golden期待値、署名済みRelease生成、公開、導入までを通します。

## 互換性

初版は`0.1.0`です。`0.1.x`の範囲では非互換な変更をしません。`0.2.0`のようにマイナーを上げるときは非互換な変更を許容し、その内容と対処を変更履歴に必ず書きます。

安定として扱うのは、日常利用のコマンド、保存される記録の形、プロジェクト内のファイル、機械可読な出力、MCPの道具、配布の形です。

移行機能、Detector品質測定、Framework検出Catalogの形式は実験的です。どの版でも予告なく変わります。人向けの表示文とRust crateのAPIは、そもそも約束の対象外です。

詳細は`COMPATIBILITY.md`にあります。Detectorの改善で検出結果が変わり、これまで進めた変更が止まることがありますが、これは非互換な変更としては扱いません。

## 報告と貢献

- 不具合の報告と提案はIssueへ。様式は用意してあります
- 脆弱性は公開の場に書かず、GitHubの非公開報告を使ってください。対象と対象外は`SECURITY.md`に書いてあります
- 変更を送る前の確認事項は`CONTRIBUTING.md`にあります。意図的に採らない方針もそこに書いてあります
- 変更履歴は`CHANGELOG.md`です

## ライセンス

MITライセンスとApache License 2.0の二本立てです。利用者はどちらかを選べます。全文は`LICENSE-MIT`と`LICENSE-APACHE`にあります。

このKitへ意図的に送った貢献は、追加の条件なしに同じ二本立ての条件で利用されます。

配布するバイナリは依存ライブラリを静的に含みます。依存ライブラリの表示義務を満たすため、リリース物には第三者ライセンス表記を同梱します。表記は次で生成します。

```sh
for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu \
              x86_64-apple-darwin aarch64-apple-darwin x86_64-pc-windows-msvc; do
  cargo fetch --locked --target "$target"
done

python3 scripts/collect-third-party-notices.py \
  --lock Cargo.lock \
  --output THIRD-PARTY-NOTICES.md \
  --target x86_64-unknown-linux-gnu \
  --target aarch64-unknown-linux-gnu \
  --target x86_64-apple-darwin \
  --target aarch64-apple-darwin \
  --target x86_64-pc-windows-msvc
```

対象を指定すると、そのプラットフォーム向けバイナリが実際にリンクするパッケージだけを収録します。`Cargo.lock`には、どの構成でも使われないパッケージも含まれるため、指定なしでは配布しないものまで要求してしまいます。
