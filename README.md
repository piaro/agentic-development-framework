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

`--dry-run`で変更予定だけを確認できます。通常は既存ファイルを上書きせず、CLIとinstallation metadataだけを更新します。v1から更新する場合は`--upgrade`を指定すると、kit管理のSkillと`AGENTS.md`管理ブロックも置き換えます。Contractやプロジェクト文書は置き換えません。

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
2. `$agentic-contract`が`.agentic/changes/<id>/contract-assessment.yaml`を作る。
3. 未解決の全体判断、Contract不足、Platform未知、競合があれば開発を止める。
4. `agentic contract resolve <id>`がaccepted Contractを固定する。
5. `agentic change ready <id>`がfresh lockとMutation競合を確認する。
6. `$agentic-builder`がresolved Contractに従って実装する。
7. `$agentic-challenger`がraw diff、Mutation Graph、Operation Contract、証拠から独立反証する。
8. `agentic evidence check <id>`で契約項目と証拠、残存リスクを検査する。
9. 障害後は`$agentic-learning`で知識を上位Contract、probe、test、runtime checkerへ昇格する。

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
.agentic/bin/agentic contract resolve <change-id>
.agentic/bin/agentic mutation build
.agentic/bin/agentic change ready <change-id>
.agentic/bin/agentic evidence check <change-id>
```

CLIはSchema、参照、status、hash、未解決事項、Mutation競合、証拠不足を検査します。Feature-localか全体判断か、業務上どの選択肢が正しいかは自動決定せず、Skillが選択肢と影響を整理し、人へ必要な判断だけを提示します。

## Levels

| Level | 導入内容 |
|---|---|
| Lite | Project / Feature Contract、Decision、手動Readiness |
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
