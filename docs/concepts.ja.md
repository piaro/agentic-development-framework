# 考え方と使い方

Agentic Development Frameworkの考え方、Contractの階層、データ整合性の扱い、日々の進め方を日本語で説明します。

入口は英語の[`README.md`](../README.md)です。対応範囲と制限は[`docs/limits.md`](limits.md)、約束する範囲は[`COMPATIBILITY.md`](../COMPATIBILITY.md)、実装の詳細は[`implementation.md`](implementation.md)にあります。内容が食い違う場合は英語版を正とします。

## 何を解こうとしているか

エージェントに実装を頼めば、何かは実装されます。確実にできないのは、必要な仕様がそもそも書かれていないと気づいて止まることです。もっともらしく、見えない形で、判断とは見えないコードとして埋めてしまいます。

埋めるべきなのはエージェントではありません。タスクを削除したとき添付ファイルも消すのかは、プロダクトの判断です。エージェントは問いがあることを見つけ、選択肢を並べ、推奨を示せます。決める立場にはなれず、指示文をどれだけ工夫してもそこは変わりません。

そして誰かが決めたとしても、その答えは終わってしまう会話の中に残ります。次の変更はまた同じことを尋ねるか、尋ねないまま別の答えを出します。

## Contractが中心にある

Contractは「いまこのRepositoryで正しいとしていること」です。規範そのもの、それを決める権限を持っていたのは誰か、何をもって満たされたとするか。この3つを持ちます。

この仕組みで残る成果物はContractです。発行される作業も、役割の分離も、各種の検査も、Contractを作り、使い、健全に保つために存在します。

**人とエージェントが一緒に作ります。** エージェントがコードを調べ、規範が足りないことを見つけ、選択肢と影響を並べ、推奨を示します。人が決めます。その判断が記録され、そこから決まった規範が、その判断を根拠として持つContract条項になります。どちらか一方では成立しません。エージェントは決められず、人は問いを一から組み立て直さなくて済みます。

**すべての変更がContractに対して行われます。** 実装前に適用するContractを確定して反証し、実装中はそれを満たすべき対象とし、実装後は各条項に追跡できる証拠が揃って初めて完了とします。

**そしてContractは育ちます。** 一度答えた問いは戻ってきません。次の変更は、尋ね直すのではなくその条項に対して解決します。1つの機能を超えて効く規範だと分かれば、上位のContractへ上げます。障害から得た知見は、テストを伴う条項になります。変更が終わるたびに、Repositoryは自分自身について前より多くを知っている状態になります。この蓄積そのものが成果です。

そうしない場合に起きることは、たいてい決まっています。知識はその場にいた人の中にだけ残り、エージェントは毎回それを推測し直し、両者の答えが食い違います。

## それを信頼できるようにする仕組み

Contractを作る意味があるのは、その過程を言いくるめられない場合だけです。作業は1件ずつ発行され、エージェントは次にやることを受け取り、それを実行し、結果を提出して、また次を受け取ります。この各段階を決めるのは、エージェントの記憶ではなく制御基盤です。

- **作業の順序は計算されるもので、指示文に書かれていない。** Kernelが記録から次の作業を導き、その識別子は導出元のdigestになる。手順を飛ばすことも、状態が求めていない手順を作り出すこともできない
- **権限は検査される性質で、判断ではない。** 要件を満たしたと報告するには、acceptedなContractの明示条項、依頼に明示された要求、記録された人の判断、acceptedなDecisionのいずれかが要る。エージェント自身の推論は証拠であって権限にはならない
- **コードが何をするかは、説明ではなく読み取る。** 16言語を構文解析し、説明できない呼び出しは変更を止める。名前だけでは分類しない
- **Contractが古くなるのは機能である。** 各結果は根拠のdigestに縛られ、Contract、コード、根拠が変われば、それに依存していた作業は古いものとして再実行を求められる。コードから乖離したContractが黙って通り続けることはない
- **自分の作業を自分で検証しない。** 実装と反証は別の役割で、実装後の反証は実装した文脈から独立して行う

最後に境界です。制御基盤が検査するのは、構造、参照、状態、digest、coverageです。意味は判定しません。誤ったことを書いたContractは受理されます。何を検証しないかを正確に知れることが、残りを信頼できる理由になります。

## 全体の動き

すべての変更は、概ね次のループを通ります。

```text
Issue・依頼・既存Docs
        │
        ▼
   adf change init
        │
        ▼
┌─▶ adf next ─── 次にやること1件を返す
│       │
│       ├─ Analyst   ─▶ 検出候補の確認、影響範囲と操作境界の確定、Contract記入
│       │                 └─ 未決定 ─▶ 人の判断 ─▶ Decision・Contractへ記録
│       ├─ Builder   ─▶ 実装、Contract条項に対応する証拠の記録
│       └─ Challenger ─▶ 実装前・実装後の反証（独立した文脈）
│       │
└───────┴─ adf submit ─── 結果を検証・保存し、再評価する
        │
        ▼
     完了判定
```

エージェントがSkillの実行順を覚えるのではなく、`adf next`が変更の状態から次の1件を決めます。エージェントはそれを実行して結果を提出し、また次を受け取ります。

役割ごとに使うSkillは3つです。

| 役割 | Skill | 担当する作業 |
|---|---|---|
| Analyst | `$adf-analyst` | 検出候補の確認、影響範囲と操作境界の確定、Contract記入、人への判断依頼、回答の記録 |
| Builder | `$adf-builder` | 実装と証拠の記録 |
| Challenger | `$adf-challenger` | 実装前と実装後の反証 |

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

変更ごとの記録は`.adf/changes/<id>/`に残ります。現在有効な規範の正本は`contracts/`、判断履歴の正本は`decisions/`です。

## 動作の前提となるDocsと情報源

Kitは、エージェントの推論だけを仕様の根拠にはしません。変更開始前に、少なくとも次の情報へアクセスできる状態にします。

| 情報 | 役割 | 仕様を決めるauthorityになれるか |
|---|---|---|
| `AGENTS.md` | Repository固有の作業規約、禁止事項、検証方法 | 仕様を決める根拠にはしない |
| Issue・要求文 | 変更の目的、明示要求、非対象、受入条件 | 明示された要求は`issue-requirement`として可 |
| `contracts/` | 現在有効な規範 | acceptedな明示clauseは`accepted-contract`として可 |
| `decisions/` | 判断理由と変更履歴 | accepted Decisionは`accepted-decision`として可 |
| 記録された人の判断 | 選択された仕様と判断者 | `human-decision`として可 |
| `docs/adf/source-of-truth.md` | 既存文書とContractの対応、正本の所在 | 索引。参照先のauthorityを置き換えない |
| コード・テスト | 現在の実装事実、回帰証拠 | 単独では不可 |
| `evidence/`・`probes/` | Platform能力と検証結果 | 事実の証拠。単独では新仕様のauthorityにしない |

Agent推論、Challenger finding、Contract gap、実装都合、既存コードだけ、テストだけでは、新しいプロダクト仕様を決定できません。

導入時に生成されるDocsは次の役割に限定します。

| Docs | 内容 |
|---|---|
| `docs/adf/README.md` | 導入先Repositoryでの運用入口 |
| `docs/adf/source-of-truth.md` | 現在の正本と既存文書の対応表 |
| `docs/adf/adoption-report.md` | 既存実装の調査結果、差異、移行候補、未確認事項 |

`docs/`は利用案内と索引です。現在有効な規範を`docs/`だけに閉じ込めず、`contracts/`へ昇格します。

## Development flow

### 基本フロー: 通常の機能変更

1. `adf change init <id>`で変更を作る。
2. `adf next <id>`が次にやること1件を返す。以降はこれを繰り返す。
3. Analystが、検出された候補を実際のコードと突き合わせて採用または除外し、影響するデータと操作境界を確定する。必要な規範が無ければContractへ記入する。
4. 既存の権限ある根拠で決められない判断が出たら、選択肢、影響、推奨、必要な決定者を添えて人へ戻す。人が答えたら、理由をDecisionへ、現在の規範をContractへ記録する。
5. Challengerが実装前に、依頼、権限、判断、提案されたContractを反証する。
6. 実装前に必要な項目がすべて満たされると、Builderへ実装が割り当てられる。
7. Builderが実装し、Contract条項に対応する証拠を記録する。実装中に新しい仕様判断が出たら、実装を止めて分析へ戻す。
8. Challengerが実装後に、変更差分、データ不変条件、テスト、証拠を使って独立に反証する。
9. すべて満たされると完了できる状態になる。

判定の理由は`adf explain <id>`で確認できます。

検出された候補の一致は、意味上の適用を自動確定しません。名前が似ているという理由で採用せず、実際のコードを読んで判断します。

### ユースケース: 仕様判断が足りない

既存の権限ある根拠から一意に決められない場合、問い、選択肢、影響、推奨、必要な判断者を判断依頼としてまとめます。変更は`needs-human-decision`で止まり、人が答えるまで先へ進みません。

```sh
adf next <change-id>
adf explain <change-id>
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

影響の大きい変更では、作成・更新・削除・再試行、並行実行、順序逆転、commit前後の停止、event重複、migration混在といった順序を試し、各操作後にInvariantを検査します。反証の観点は`adf-challenger`のSkillにある参照文書にまとめてあります。

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

反証で新しい意味を推測で追加することはしません。反例が示すContractの不足は、分析へ戻して判断依頼にします。観点は`adf-challenger`のSkillにある参照文書にまとめてあります。

## Repository内の情報配置

| 場所 | 役割 |
|---|---|
| `contracts/` | 現在有効な規範（What） |
| `decisions/` | 規範を決めた理由と変更履歴（Why） |
| `docs/adf/` | 利用方法、既存正本との対応、導入報告（How / Index） |
| `probes/` | 外部Platformを実証する実行物 |
| `evidence/` | Contract clauseに対応するテスト、probe、反証、残存リスク |
| `.adf/changes/<id>/` | 変更ごとの記録、発行された作業の結果、判断依頼 |
| `.adf/config.yaml` | 正本の場所と観測結果の位置 |
| `.adf/framework.lock` | 使用するFramework Releaseの固定 |
| `.adf/repository-observation.yaml` | コード上の物理識別子と論理IDの対応付け |
| `.agents/skills/` | エージェント向けSkill。`project init`が配置する |

## Shell・CLI・エージェント・人の責務

| 担当 | 責務 |
|---|---|
| 制御基盤 | 次にやることを決め、構造、参照、状態、hash、coverage、競合、証拠不足を機械検査する |
| Agent Skills | Repository調査、Contract記入、選択肢整理、実装、意味的な反証を行う |
| 人 | 権限ある根拠のないプロダクト判断、risk受容、組織的な優先順位を決定する |

人へすべてのgapを丸投げするのではなく、エージェントは既存authorityから解決できるものを処理し、未決定の問いだけを選択肢、影響、推奨とともに提示します。
