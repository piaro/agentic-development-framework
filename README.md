# Agentic Development Kit

AIエージェントを前提に、現在有効な正しさを階層Contractとして解決し、未決定事項を実装前に止め、データ操作の組み合わせを独立に反証するRepository内コントロールプレーンです。

コードレビューを増やすことではなく、Contract Readiness、Platformの実証、Mutation Graph、Builder / Challenger分離、証拠による完了判定、不具合からの学習を配置します。

## Quick start

```sh
./bin/agentic-init
```

```sh
./bin/agentic-init \
  --mode adopt \
  --level system \
  --target /path/to/existing-project \
  --non-interactive
```

`--dry-run`で変更予定だけを確認できます。通常は既存ファイルを上書きせず、CLIとinstallation metadataだけを更新します。導入済みRepositoryを更新する場合は、最新版のkitから次を実行します。

```sh
./bin/agentic-init --upgrade --target /path/to/project --non-interactive --dry-run
./bin/agentic-init --upgrade --target /path/to/project --non-interactive
/path/to/project/.agentic/bin/agentic --version
```

`--upgrade`は導入先の`.agentic/installation.yaml`からmode、level、project名を引き継ぎ、kit管理のCLI、Skill、`AGENTS.md`管理ブロック、installation metadataだけを更新します。Contract、Assessment、設定、プロジェクト文書は置き換えません。

3.0.0ではContract decisionのauthorityと実装前Challengeを必須化しました。2.xからのupgrade後は、active changeに対して`agentic contract authority-check --all`を実行し、Assessmentを手動で移行してから再Challenge・再resolveしてください。旧resolved lockは新しいreadinessには使用できません。

### 2.xから3.0.0への移行

1. `--upgrade --dry-run`でKit管理物だけが更新されることを確認し、`--upgrade`を実行する。
2. `agentic contract authority-check --all`でactive changeのAssessment、Feature対応、Challenge、lockを診断する。
3. activeな`contract-assessment.yaml`をschema version 2へ更新し、resolved/accepted decisionへauthorityを追加する。意味を判断できないbackfillは行わず、Decision Requestにする。
4. resolvedな仕様拡張decision IDをFeature Contractの`introduced_decisions`へ追加する。
5. `agentic contract challenge-input <id>`の出力を基に実装前Challengeを行う。
6. Challenge通過後に`contract resolve`と`change ready`を再実行する。

completed/cancelled changeの履歴は移行対象外です。`--upgrade`は既存のAssessment、Contract、Decision、schema file、設定、docsを上書きしないため、新しい構造の強制は更新されたCLIが行います。

## 情報の役割

| 場所 | 役割 |
|---|---|
| `contracts/` | 現在有効な規範（What） |
| `decisions/` | 規範を決めた理由と変更履歴（Why） |
| `docs/agentic/` | 利用方法、既存正本との対応、導入報告（How / Index） |
| `probes/` | 外部Platformを実証する実行物 |
| `evidence/` | Contract clauseに対応するテスト・probe・反証・残存リスク |
| `.agentic/` | 変更状態、Assessment、resolved lock、Mutation Graph |

## Contract hierarchy

```text
Project Contract
  ├─ Domain Contract
  ├─ Capability Contract
  ├─ Architecture Contract
  ├─ Data Invariant
  └─ Operation Contract
         ↓
     Feature Contract（今回の差分）
```

Feature Contractは上位Contractを暗黙に上書きしません。所有権、多重度、状態遷移、Protocol、保存形式、共通Platform利用などが未決定なら、Featureをblockedにして上位Contractを先に決めます。

## Development flow

```sh
.agentic/bin/agentic change init feature-id --title "変更タイトル"
```

1. `$agentic-change`が影響するContext、Entity、Capability、Operation、Interface、Pathを復元する。
2. `agentic contract candidates <id>`で単一軸一致を候補として列挙する。
3. `$agentic-contract`が候補、decision、authority、gapをAssessmentへ記録する。既存authorityで決められなければDecision Requestを作り、`blocked-contract-decision`にする。
4. `$agentic-challenger`が実装前にIssue、authority、decision、Featureと上位Contractを反証する。findingはauthorityにしない。
5. 未判断候補、authority不足、未解決Decision Request、Contract coverage不足、stale/blocked Challenge、Platform未知、競合があれば開発を止める。
6. `agentic contract resolve <id>`がaccepted Contract、authority検証、Challenge結果、除外判断を固定する。
7. `agentic change ready <id>`がfresh lockとMutation競合を確認する。
8. `$agentic-builder`がresolved Contractに従って実装する。新しい仕様判断を発見したらAssessmentへ戻す。
9. `$agentic-challenger`が実装後にraw diff、Mutation Graph、Operation Contract、証拠から独立反証する。
10. `agentic evidence check <id>`で契約項目と証拠、残存リスクを検査する。
11. 障害後は`$agentic-learning`で知識を上位Contract、probe、test、runtime checkerへ昇格する。incident findingだけで新仕様を決めない。

候補一致は意味的な関連を自動確定しません。Project、Featureの明示参照、Operation/APIの厳密一致は必須契約とし、それ以外のContext、Entity、Capability、Pathなどの一致はAssessment対象にします。除外には理由が必要で、候補の追加、適用条件の変更、除外契約の内容変更はresolved lockをstaleにします。

## Authority and Decision Requests

`reason`は判断理由であり、仕様を決めるauthorityではありません。resolved/acceptedなAssessment decisionは、既存accepted Contractの明示clause、Issueの明示要求、記録された人の判断、既存accepted Decision recordのいずれかを参照します。Agent推論、Challenger finding、Contract gap、実装都合、コード、テストは証拠として記録できますが、単独のauthorityにはなりません。

既存authorityから一意に決められない場合は、Assessment decisionを`needs-human-decision`にし、問い、選択肢、影響、推奨、必要な判断者を一時的なDecision Requestとして記録します。未解決中は`blocked-contract-decision`です。解決後の正本は`decisions/`の判断履歴と`contracts/`の現在値であり、後続artifactはDecision Requestへ依存しません。

```sh
.agentic/bin/agentic contract decisions <change-id>
.agentic/bin/agentic contract decisions --all --format markdown
.agentic/bin/agentic contract challenge-input <change-id>
.agentic/bin/agentic contract authority-check --all
```

## Data integrity

Data Invariantは操作順序に依存せず守る状態、Operation Contractは各writeのprecondition、mutation、transaction、external effect、postcondition、consistency、idempotency、failure pointを定義します。

```sh
.agentic/bin/agentic mutation build
```

Operation ContractからEntity writerとactive change競合を生成します。System以上ではScenario Driverを接続し、create/update/delete/retry、並行実行、順序逆転、commit前後の停止、event重複、migration混在を試し、各操作後にData Invariantを検査します。

## CLI responsibility

Python 3.10以上とPyYAML 6系を使用します。

```sh
python3 -m pip install -r .agentic/runtime/requirements.txt
.agentic/bin/agentic contract lint
.agentic/bin/agentic contract candidates <change-id>
.agentic/bin/agentic contract decisions <change-id>
.agentic/bin/agentic contract challenge-input <change-id>
.agentic/bin/agentic contract authority-check --all
.agentic/bin/agentic contract resolve <change-id>
.agentic/bin/agentic mutation build
.agentic/bin/agentic change ready <change-id>
.agentic/bin/agentic evidence check <change-id>
```

CLIはSchema、参照、status、hash、未解決事項、Mutation競合、証拠不足を検査します。Feature-localか全体判断か、業務上どの選択肢が正しいかは自動決定せず、Skillが選択肢と影響を整理し、人へ必要な判断だけを提示します。

## Levels

| Level | 導入内容 |
|---|---|
| Lite | Project / Feature Contract、authority、Decision Request、実装前Challenge |
| Standard | Domain / Capability / Architecture、Data Invariant、Operation Contract、適合性検査 |
| System | resolver、Mutation Graph、active change競合、Platform probe、Stateful Challenger |
| Critical | Threat / Failure Model、Failure Injection、runtime invariant、reconciliation、release evidence |

上位レベルは下位を含みます。データを持つ継続運用アプリはStandard、複数サービス・非同期・複数エージェントはSystemを基準とします。個別変更はR0-R3で一時昇格できます。

## Shell and agent responsibilities

Shellが行うこと:

- レベル別ファイル、CLI、Skillの配置
- `AGENTS.md`への管理ブロック追加
- 新規モードでのGit初期化
- 既存ファイルを上書きしない冪等な再実行

エージェントが行うこと:

- 既存コード、仕様、Issue、判断履歴の調査
- 影響範囲と全体判断候補の抽出
- Contract Readiness Assessment
- Data Invariant、Operation Contract、Platform probeの意味設計
- Builder / Challengerとしての実装と独立反証

## Validation

```sh
sh -n bin/agentic-init
python3 -m py_compile bin/agentic
sh tests/test-init.sh
python3 /path/to/skill-creator/scripts/quick_validate.py skill-src/<skill-name>
```
