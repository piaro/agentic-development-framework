# FRAMEWORK-REVIEW 設計レビュー

> 状態: レビュー結果
>
> レビュー日: 2026-07-29
> 対象: `FRAMEWORK-REVIEW.md` 全体、`prototype/vnext/` のPython・Rust実装、共有golden fixture、`tests/test-vnext-*`
>
> この文書はレビュー結果であり、確定した設計変更ではない。対応方針は別途決定する。

## 1. 結論

指摘は2層ある。

**根本設計の層**では、フレームワークが何を保証しているのか、その保証を成り立たせる部分がどこにあるのかが、目的と食い違っている。最も重いのは次の2点である。

- Shared Contractの変更間再利用とlost update防止は追加されたが、進行中Changeどうしの事前競合検出と条項競合の解決経路は未決定である
- Signal依存の制御の起点となる検出処理はcoverage未報告を停止できるようになったが、観測対象自体の記載漏れと、実装上の対象を導入先IDへ対応付ける規則は未解決である

**個別設計と実装の層**では、通常のbuild後に工程全体をやり直す状態になる問題が最も重い。

中心にある判断そのもの、つまり工程の管理をエージェントの記憶から実行プログラムへ移す方針、確認事項の定義と適用条件と検出処理を別々のIDで管理する分離、優先順位ではなく和集合と衝突検出でルールを合成する方式は、いずれも妥当である。問題は、その方針を成立させる部分がまだ空いていることと、空いている部分の周辺が先に作り込まれていることである。

## 2. 根本設計への指摘

### 2.1 Shared Contractの再利用とlost update防止は確認できたが、active change競合は扱っていない

2.1 が挙げる問題の層は4つあり、3番目の「全体整合性の喪失」、つまり判断が変更・セッション・担当者を越えて伝播しないことが、このフレームワークの主目的である。2章の冒頭も「AIエージェントや複数の開発者が並行して変更を行う開発」を前提に置いている。

初回レビューでは「Prototypeは単一の変更しか扱わず、Shared Contractも存在しない」としたが、この記述は正確ではなかった。ContractとDecisionの`change_id`はSchema上任意であり、`project.py`の`_by_change`は`change_id`のないRecordをすべてのChangeへ含める。つまり、`change_id`を持たないRecordがShared Contractまたは共有Decisionとして働く基礎機構は既にあった。

対応として2 Changeのfixtureとテストを追加し、現在は次を確認できている。

| 確認項目 | 現在の状態 |
|---|---|
| 変更が2件あるfixture | `db-sqs/project.yaml`と共有golden fixtureに追加した |
| Shared Contract | `change_id`を持たないContractとDecisionを追加し、Python・Rust双方で2つ目のChangeから見えることをgolden testで確認した |
| Change Contractの隔離 | `change_id`付きContractとDecisionは、別ChangeのSnapshotおよびGenerated Contextへ入らないことを確認した |
| 判断の再利用 | Change Aで人が確定した内容をShared Contractへ記録し、Change BがそのContractを根拠に人への再質問なしでChallengerまで進む縦断テストを追加した |
| Kernelの視野 | 一回の評価対象は依然として1 Changeと、それに適用される共有Recordである。ほかの進行中のChange本体は見えない |
| Shared Contractのlost update | 更新時に読取り時のContract digestを要求し、正本の排他lock内で現在値と比較する。先に更新された場合は後続更新をstaleとして拒否する |
| 並行する変更どうしの事前競合 | 未実装である。1282行の`active changes`は「Change群からKernelが生成する索引」とあるだけで、競合検出の意味論ではない |

これにより、変更をまたぐ問題を次の2つに分けて扱う必要があることが明確になった。

| 問題 | 評価 |
|---|---|
| 確定した判断を後続Changeへ伝播する | Shared Contractで表現・再利用できることを確認済み |
| 同時進行のChangeによるShared Contractの上書き消失を防ぐ | digestによる楽観的並行制御で確認済み |
| 同じ条項への異なる変更案を検出し、人へ戻す | 未解決 |

現行KitはSystem Levelに「active change競合」を持っていたが、vNextではまだ対応する制御がない。README 95行を参照。

また、Shared Contractへの昇格をApplicationが内容から自動判断してはならない。現在のテストは、Analystが人の回答の再利用範囲を選び、`change_id`なしで記録する経路を表している。実運用では、記録Actionが次を明示的に入力として受け取る必要がある。

- Change固有として記録するか、Sharedとして記録するか
- その再利用範囲を選んだ根拠と決定権限
- Sharedへ記録する場合の対象Contractと条項ID

Shared Contract更新時に読み取ったContract digestを要求する楽観的並行制御を実装した。Python・Rust双方で、Project Adapterが排他lock内で現在値を再読込みし、digest一致時だけatomic replaceする。digestの省略と不一致は正本を書き換えず拒否し、同じ旧digestを基にした二つの更新では先に保存された一件だけが残る。

残る設計判断は、stale拒否後の意味的な解決である。現状は呼出し側が最新Snapshotから再評価するところまでを規則とし、条項単位の自動mergeは行わない。同じ条項へ異なる変更案がある場合にHuman Authorityへ戻す経路と、保存を試みる前にactive Changeの重なりを表示する索引は、別途設計する必要がある。

### 2.2 検出処理の入力が、フレームワークの統制外にある

Signalを条件とする制御はrisk signalの検出から始まる。検出できなければ、そのSignalに依存するRequirementは選ばれず、何も止まらない。Human AuthorityなどSignalに依存しない制御は残るため、「全ての制御」とする初回の表現は過大だった。

現在の検出処理の入力を確認した。`fixtures/cli-project/.agentic/repository-observation.yaml` が、コードの成果物と導入先固有の論理IDとの対応、および事実そのものを手書きで宣言している。

```yaml
artifacts:
  - ref: code.place-order-handler
    path: src/place_order.py
    applies_to: [operation.place-order, data.orders]
facts:
  - kind: db_write
    operation: operation.place-order
    data: data.orders
    evidence_refs: [code.place-order-handler]
```

対応する `src/place_order.py` は次の内容である。

```python
def place_order() -> str:
    """Persist an accepted order in the source-of-truth table."""
    return "stored"
```

つまり、検出処理は現在、**手書きの答えを型に変換しているだけ**である。14.11 が「実コードからのsignal検出」を未達成の一項目として挙げているが、これは一項目ではなく、フレームワーク最大の設計問題である。

難所は「データベースへの書込みを見つける」ことではない。**その書込み先を、導入先が所有する語彙で名前付けする**ことである。`orders_table.insert()` が `data.orders` であると、誰がどう決めるのか。取り得る方式は次の4つで、性質が大きく異なる。

| 方式 | 利点 | 問題 |
|---|---|---|
| コードへ注釈を書く | 位置が正確 | コードへの侵入。注釈の腐敗を検出できない |
| 対応表を保守する | コードを変えない | 対応表自体が腐敗する。誰も更新しなくなる |
| Analystが毎回写像する | 保守物が増えない | 実行ごとに結果が変わる。14.8 の確認結果の再利用が成立しなくなる |
| コード構造からIDを導出する | 保守物が不要 | Contractが実装の名前に依存する。所有の向きが逆転する |

設計はこの比較を行っていない。既定では3番目、つまりAgentが毎回写像する形になるが、それを選ぶと確認結果を再利用する仕組みが壊れる。

さらに問題なのは、当初の`repository-observation.yaml`に**完全性、決定権限、意味上の鮮度検査がなかった**ことである。この一つのファイルが誤っていれば、Signal依存の制御を静かに迂回できた。フレームワークの統制が最も弱い入力であり、誰の権限で業務IDへ対応付けたか分からない暗黙の判断だった。

確認の結果、manifestはGit管理され、artifact bytesと宣言metadataのdigest変更も候補fingerprintへ反映されるため、完全に統制外という表現も正確ではなかった。一方で、`facts: []`にするとSignal依存Requirementが一件も生成されず、pre-buildでは`ready-to-build`、post-buildでは`ready-to-merge`へ進むことを再現できた。Git管理とdigestは、書かれた内容の変更は検出するが、書くべき内容の欠落は検出しない。

対応として、Detection Reportへ次を追加した。

- `coverage.status`: `complete`または`incomplete`
- `coverage.scope`: Detectorが解析した範囲の種別
- `coverage.analyzed_refs`: 解析済みartifact
- `coverage.gaps`: parse失敗、未解析artifact、未対応言語、`unmapped-observation`など

最初の対応では、coverage未報告を`coverage-not-reported`として`incomplete`にし、Kernelを`blocked-detection`で停止させた。coverageにgapがある場合も同様に停止し、空のfact一覧だけでは完了扱いにしない。未知のfact kindも候補なしとして無視せず、Detector入力エラーにした。Git AdapterはObservation Schema v2でcoverage宣言を必須とし、宣言済みartifactのうち`analyzed_refs`にないものを`unscanned-artifact`として補った。

この段階のcoverage scopeは`declared-artifacts`であり、manifestからartifact自体を記載し忘れた場合や、手書きfactの意味が誤っている場合までは検出できなかった。したがってv2はfail-closedなcoverage protocolの導入に留まり、根本問題の完了ではなかった。

正式方式は次の3層に分ける。

1. language-specific DetectorがGit差分とsourceから、symbol・呼出先・物理resourceを持つ観測候補と解析範囲を生成する
2. 導入先が所有するBinding Recordが、観測候補を`data.orders`などの安定IDへ対応付ける。Agentは候補を作れるが、根拠・所有者・確定権限を持つRecordだけを再利用する
3. 未対応言語、parse失敗、未対応観測、binding未解決、解析対象漏れをcoverage gapとしてKernelへ渡し、解消または権限付きの明示的Decisionなしには進めない

Binding Recordをartifact単位で先に固定すると、同じfile内の複数symbolを区別できなくなる。そのため、次の実装ではGit差分から得る解析単位とsymbol identityを先に決め、その後にBinding Record Schema、鮮度規則、生成Actionを追加する。手書きの`repository-observation.yaml`はそれまでのPrototype fixtureに限定する。

Rust版の最初の縦断実装として、Observation Schema v3へ移行した。`facts`と`coverage`の手書き入力は廃止し、Rustのlanguage-specific DetectorがTree-sitterでPython構文を解析する。解析対象は`analysis.roots`配下にあるGit管理済み・未追跡の`.py`であり、manifestにないsourceも`unbound-source-artifact`として停止する。文字列やコメント内の語句ではなく、関数内のmethod callからDB書込みとmessage publishを観測する。

Binding Recordはartifact単位の`applies_to`ではなく、関数名と物理resource名を別々に論理IDへ対応付ける。各対応は`owner`と`authority_ref`を必須とし、承認先は現在のChangeで`accepted`になっているDecisionでなければならない。artifact bytes、Binding Record、承認Decisionはいずれも候補の検出根拠に含まれるため、変更後は除外判定をそのまま再利用しない。parse失敗、未対応言語、未登録source、binding未解決、Binding済みresourceへの未対応methodはcoverage gapとして`blocked-detection`へ送る。

この実装ではLLMを解析器として使わない。LLMはBinding Recordの候補作成を支援できるが、通常評価が再実行のたびに意味写像を推測する構成にはしない。現時点のlanguage-specific DetectorはPythonの限定したmethod群だけであり、alias、動的dispatch、framework固有API、他言語はDetectorを追加してcoverage範囲を広げる必要がある。

その後、Observation Schema v4で言語非依存の観測形式とDetector登録表を追加し、Pythonに加えてJava、Go、Rust、JavaScript、JSX、TypeScript、TSXを同じGit Adapterへ接続した。Kotlinなどの主要source拡張子もinventory対象とし、Detector未実装の言語を初期観測から黙って除外しない。framework固有APIは`resource.method`単位のBindingへ`db_write`または`message_publish`、owner、承認Decisionを記録することで追加できる。これによりSQLAlchemyの`session.execute`、Djangoの`model.save`、Spring Dataの`repository.save`を導入先の判断で分類できるが、alias、動的dispatch、query内容の意味解析は引き続きcoverage外としてfail closedに扱う。

### 2.3 保証しているのは「問うたこと」であり「確認されたこと」ではない

13.8 の末尾は「Kernelは設計や実装が意味的に正しいかを判定しない。必要なResult、参照先、入力内容の一致、検証範囲、承認、Evidenceが揃っているかだけを機械的な進行条件とする」と正直に書いている。これ自体は正しい。

問題は、この性質の帰結が設計に反映されていないことである。

各Requirementが充足したと判定される条件を確認した。`schemas/v1/result-payloads/*.schema.json`によれば、必要なのは結論の状態、1文字以上の要約、発行済みContext内に実在する根拠参照である。つまり、**形式の整ったResultを出せば充足する**。実際に調査したかどうかは検査していない。

すると 3.1 が指摘した問題は解消せず、形が変わるだけになる。

```text
これまで: Agentが確認手順を忘れる
これから: Agentが形だけ整えた浅い確認を提出する
```

設計上の答えはChallengerによる独立した反証だが、Challengerの出力も同じ形式検査しか受けない。反証する主体を反証する仕組みは無く、原理的にも置けない。

13.6 はEvidenceへ「実行したコマンド、終了結果、検証したGit revision、テストレポートへの参照」を残すとしている。ただし、Recordへ項目が存在することと、信頼できる実行主体が実際に検証したことは同じではない。Agent自身がEvidenceを書ける場合、項目を増やすだけでは自己申告の形式が詳しくなるだけである。

したがってRequirementは、少なくとも性質の異なる2種類を区別する必要がある。

| 種別 | 保証内容 | 例 |
|---|---|---|
| `attestation` | 指定Roleが、発行済みContextに対して根拠付きの充足申告を提出したこと | `affected-data-confirmed`、設計内容の妥当性、各Challenge |
| `evidence-backed` | 現在revisionとRequirement Instanceに対応する成功Evidence Recordがあり、必要なContract条項を覆っていること | 条項対応テスト、Schema検査、再現可能な実行結果 |

Rust版へ次の区別を実装した。

- Requirement定義の`assurance`を省略した場合は`attestation`とする。既存定義の保証を暗黙に強く解釈しない
- `evidence-backed`は`result.evidence`だけに許可する
- Rust Kernelは、Resultの充足申告だけでなく、Resultが参照するEvidenceのRequirement Instance、現在のGit revision、成功結果、Contract条項coverage、実行方法、終了コード、Artifact URIとdigestを検査する
- 発行後に追加したEvidenceはResultの`output_refs`として取り込み、outcome単位の鮮度参照へ含める
- Evidence Recordは追記専用とし、同じIDを上書きしない

`data-contracts-ready`全体を`evidence-backed`へ移してはならない。「条項の内容が十分か」は意味判断なので`attestation`に残し、「各条項に対応する検証が現在revisionで成功したか」を別Requirementとして分離する。

再確認の結果、機構は実装されていたが、標準Ruleの`data-evidence-recorded`と`distributed-effect-evidence-recorded`に`assurance`指定がなく、既定の`attestation`として動作していた。この設定漏れを修正し、両Requirementだけを`evidence-backed`へ変更した。その他の分析・設計・Challenge Requirementは意味判断を含むため`attestation`のままとし、この分類をRust統合テストとRule Compiler goldenで固定した。

なお、今回の`evidence-backed`が保証するのは、構造化された成功Evidenceが現在の入力と対応して記録されていることまでである。実行主体まで保証するには、CI／runnerがEvidenceへ署名し、Rust側が導入先のTrust Storeで検証する境界が別途必要になる。それを実装するまでは「実際にCIで実行された」とは表現しない。

### 2.4 signalの除外が、権限の規則を迂回している

内部矛盾である。

5.5.3 は次を定める。

> Requirementを省略する必要がある場合は、対象ID、適用範囲、理由、決定権限、期限をDecision Recordに残す。省略可能と定義されていないRequirementは解除できない

一方 5.5.2 は、Analystが根拠付きでsignal候補を`confirmed`または`excluded`にできるとする。`excluded`にすると、その候補を条件とするルールが評価されず、Requirementは最初から選ばれない。

`excluded`に必要な情報を確認した。`schemas/v1/result-payloads/risk-signal-review.schema.json`では、`fingerprint`、`status`、`reason`、`basis_refs`のみで、決定権限への参照も期限も無い。

この指摘は二つに分ける必要がある。

- Detectorの誤検出やbinding不一致を非該当と分類することは、Requirementの例外解除ではない
- signalが実際に存在するのにRequirementを省略することは例外解除であり、Decision Record、決定権限、適用範囲、期限が必要である

また、Result RecordはChangeごとにProject Snapshotへ絞られるため、同じfingerprintの除外が将来の全Changeへ再利用されるという初回の記述は誤りだった。再利用範囲は同一Change内である。この段階のRust版でもDetector versionが変わればfingerprintが変わり、検出根拠digestが変わった場合はfingerprintとは別に非該当判定だけを再確認していた。静的なコード根拠だけに一律の期限を設ける必要はない。

一方、実装上の迂回経路は実在した。全候補を`excluded`にするとsignal依存のChallenger Requirement自体が選ばれず、Challengerが起動した場合も、そのContextへ除外候補と理由が渡されていなかった。

Rust版では次のように対応した。

- 保存protocolも`confirmed`または`not-applicable`とし、Requirement免除を連想させる旧`excluded`値は後方互換を残さず廃止する
- `not-applicable`候補ごとに、Kernel管理の`risk-signal-applicability-reviewed` Requirement Instanceを生成する
- Analystの除外だけでは解決済みにせず、独立したChallenger Actionを必須にする
- Challenger Contextへsignal、binding、検出根拠参照、Analystの理由・根拠参照、Disposition Result IDを渡す
- Challengerが`satisfied`とした候補だけを非該当として確定する
- `unsatisfied`または`inconclusive`なら、除外を安全側に倒して`confirmed`としてRuleを評価する
- fingerprintとevidence digestごとにInstanceを分け、新しい候補や検出根拠の変更を過去のChallengeで充足させない
- Explainでは確認前を`applicability-pending`、支持後を`not-applicable`、不支持・判断不能後を`confirmed`と表示する

signalが存在するがriskを受容してRequirementを省略したい場合は、`not-applicable`を使用せず`confirmed`にした後、5.5.3のDecision経路を使う。ただし、現行Rust PrototypeはRequirement省略をまだ実装していないため、今回の対応では省略経路を追加せずfail closedのままとする。将来実装する場合も、Decision Record、決定権限、適用範囲、期限を必須にする。

### 2.5 Contractの腐敗を検出する仕組みが無い

指摘の中心は正しいが、「古いEvidenceが有効とされる」という説明は現行Rust版には当てはまらない。

Rust Kernelの`evidence-backed` Requirementは、Evidenceの`git_revision`が現在revisionと一致し、成功結果とContract条項coverageを持たなければ充足しない。また、Resultの入力digestが変わればstaleになる。したがって、選択済みRequirementに古いEvidenceをそのまま流用する経路はない。

一方、次の欠落は実在する。

- Requirementの選択とEvidence読込みはChange単位であり、触られていないContract条項を再評価しない
- Evidence RecordはChangeに属するため、通常のProject Snapshotだけでは複数Changeの検証履歴を横断できない
- Repository全体について、どの条項が検証済み、入力変更でstale、未検証、検証失敗なのかを表示できない

Contractは「現在守るべき規範」であり、実装の状態を記録する場所ではない。このためContract自体を「古い」と分類するのではなく、条項ごとに実装の準拠状態を導出する。

Rust版へ、全ChangeのResult・Evidenceと現在のRepository観測を読み取る`ContractHealthReport`を追加した。状態は次のとおりである。

- `verified`: 成功Evidenceを参照するBuilder Resultがあり、outcomeの入力ref・digestがすべて現在値と一致する
- `stale`: 検証履歴はあるが、対応するContract、コード、設定、Evidence等の入力digestが変わった、または失われた
- `unverified`: 対応する検証履歴がない
- `failed`: 現在の入力に対応するEvidenceが`failed`または`inconclusive`である

鮮度をRepository全体のHEADや経過日数だけで判定すると、無関係な変更でも全条項がstaleになる。そのためReportは、Evidenceを受理したResultのoutcome単位`freshness_refs`を現在のartifact digestと比較する。Reportは正本へ保存せず毎回再生成し、`contract-health` CLIのtext／JSON表示から確認する。未検証を自動的に合格扱いせず、Contract本文へ検証状態を書き戻さない。

さらにApplicationは、現在選択されたRequirementのsubjectと適用範囲が一致する`stale`／`failed`条項だけをKernelへ渡し、条項ごとの組込み`contract-clause-revalidated` Requirementを生成する。このRequirementは`before-merge`、Builder、`evidence-backed`であり、Contextには対象条項本文、既存Evidence、`stale_refs`を含むhealth findingだけを投影する。無関係な条項と`unverified`条項は、この経路だけを理由にはChangeを停止しない。現在入力に対する成功Evidenceが追加されれば、過去の失敗やstale状態は解消される。

Repository全体を定期CIでどの状態まで許容するかは、個別Changeのゲートとは別の運用ポリシーとして引き続き決める必要がある。

### 2.6 Markdown正本から、必要な条項本文をContextへ渡せるか

指摘のうち「正本の保存形式」と「Contextの選択単位」は関連するが、同じ一つの判断ではない。Markdown Recordのtyped blockをSchema検証済みの内部表現へ変換すれば、保存形式と条項選択を分離できる。block外の説明、例、図は人向けであり、機械的な規範として扱わない。

実際の不足は次の2点だった。

- `applies_to`がContract全体にしかなく、同じContract内の条項をRequirementの対象ごとに絞れなかった
- Generated Contextは参照IDとdigestだけを持ち、担当者が読むべき条項本文を含んでいなかった

Rust版では、Contract clauseへ省略可能な`applies_to`を追加した。指定時は条項固有の範囲、未指定時はContract全体の範囲を継承する。Context CompilerはRequirement Instanceの`subject_refs`と有効な適用範囲が重なる条項だけを選び、条項ID、本文、適用範囲、authority参照、digest、選定先InstanceをGenerated Contextへ格納する。

条項固有の範囲を持つContractでは、source manifestも`contract-id#clause-id`単位にする。これにより、同じContractの無関係な条項変更だけでResultをstaleにしない。既存Contractは互換性のためContract全体のsourceを使い、全条項が親の`applies_to`を継承する。細粒度の鮮度判定が必要なContractから段階的に条項範囲を明示する。

### 2.7 DetectorとRuleが共有するSignal語彙の定義がなかった

5.5.2がRule条件をSchemaで型付けした事実に限定し、任意コードと自然言語判定を禁止する方針は正しい。ただし、機械観測、Rule条件、Agentによる意味確認をすべて同じ「型付き事実」とみなす必要はない。

- Detectorのrepository factは、コード等から機械的に観測する入力である
- Signalと工程は、RuleがRequirementを選ぶための閉じた条件である
- Contract条項の十分性は、AnalystやChallengerがRequirementとして確認し、Resultへ記録する

したがって、14.5の「build後の`persistent-data-write`」は既存Signalと工程の組合せであり、新しい事実型ではない。13.4の条項内容もKernelの条件語彙へ展開しない。一方、5.5.6のcache分類をRequirementの省略に使うなら、決定権限と鮮度を持つ構造化入力が必要になる。低risk属性で既存Requirementを差し引くのではなく、高risk Signalに応じてRequirementを追加する単調なRuleを基本とする。

実装上の問題は、Rust Rule CompilerがSignal名を任意の文字列として受理し、Detectorが生成しないSignalや存在しない`binding.*`を持つRuleを登録できたことである。誤記したRuleはエラーにならず、単に発火しない。

Rust版へ組込みSignal Catalogを追加し、各SignalのID、生成Detectorとversion、必須bindingを一か所で定義した。Detector出力はbinding集合をCatalogへ照合し、Rule Compilerは未知Signal、生成されないbinding参照、Signalなしのbinding参照をcompile時に拒否する。現在の語彙は`persistent-data-write`、`distributed-effect`、`message-or-event-publish`の3種類に限定する。

実装言語はRustへ一本化する。Pythonを設計探索用の先行実装にはせず、新しいSignalはRustのCatalog、Detector、Rule、Rustテストを同時に更新する。言語非依存Schemaとgolden fixtureは保存形式・protocolの互換境界として維持するが、Pythonとの同等性を新規設計の受入条件にはしない。

### 2.8 正本の既定の置き場が、4.6の原則と逆行している

4.6は「導入先が所有する情報を最小化する」を原則に置き、5.11は所有権をpathで決めるとしていた。そのうえで新規導入時の既定を`.agentic/contracts/`と`.agentic/decisions/`にしており、長期的なプロダクト知識とFrameworkの運用Recordの境界が分かりにくかった。

Rust版の新規初期化と既定のProject Storeを、Repository直下の`contracts/`と`decisions/`へ変更した。これらはFrameworkを外しても残るプロダクト固有の規範と判断履歴である。

`.agentic/`には、Framework設定・lock・Change・Result・Evidence・拡張・cacheを置く。Change等はGit管理する導入先所有Recordだが、Frameworkの進行管理protocolへ依存するため、この名前空間に置く。

ProjectConfigによるRepository相対pathの指定は、既存のADRや任意の文書配置を読むための通常機能として維持する。`.agentic/contracts/`専用の互換分岐や自動移動は追加しない。設定されていれば通常の任意pathとして読めるが、新規初期化の既定にはしない。

### 2.9 成功の定義が無い

8章は13の指標を並列に挙げていたが、主指標、制約指標、診断指標を区別しておらず、目標値と比較の基準線もなかった。

主指標を「実装開始後の仕様手戻り率」とする。評価対象Changeのうち、最初の`ready-to-build`後に、実装前から利用可能だった入力で確認できたContract不足、未決定事項、決定権限のない判断が判明し、AnalystまたはHuman Authorityへ戻ったChangeの割合である。同じChangeで複数回発生しても一件と数える。

全部のChangeを止めればこの値だけは改善できるため、次を制約指標にする。

- 重大な見逃し: データ消失、認可違反、取消不能操作等に関する実装前から確認可能なgapが`ready-to-build`を通過した件数。目標は0件
- 不要な停止率: 正解ラベル上は既存Contract・Decisionだけで進行可能なのに停止したChangeの割合。初期上限は10%
- `ready-to-build`までのactive time中央値: Human回答待ちを除く評価実行時間。現行方式の120%以内

主指標の初期目標は、同じ評価シナリオを使った現行方式の基準線から30%以上削減することとする。基準線が0件の場合は非悪化を要求する。判定は、重大な見逃し0件、主指標の改善、負担制約内、の順に行う。

Context不足、再現率、説明可能性、更新作業量等は原因分析に使う診断指標とし、単独の成功判定には使わない。「実装前に確認可能だったか」と「停止が不要だったか」はKernelが自動判定せず、正解ラベルを持つ評価シナリオまたは事後レビューで確定する。

定義と初期目標は確定したが、現行方式の基準線はまだ測定していない。基準線とRust版の比較が完了するまで、Framework全体が成功したとは表現しない。

## 3. 個別設計と実装への指摘

### 3.1 通常のbuild後、必ず工程全体をやり直す状態になる

この層で最も重い。

検出処理が出す候補の同一性は、検出根拠となったコードのhashを含めて計算している。`prototype/vnext/agentic_vnext/detection.py:35-53`を参照。一方で 13.8 と 14.6 の11番は、実装後にコード差分から候補を再検出し、未確認の候補があればAnalystへ戻すと定めている。

Builderが実装したファイルは必ずhashが変わるため、そのファイルを根拠にしていた候補は毎回「新しい未確認候補」になる。Builderは必ず対象ファイルを編集するので、この経路は常に発動する。

#### 再現手順

`advance_to_ready_to_build`まで進めた後、実装後の状態へ移す際にコードのhashを1つ変える。

```python
repo = deepcopy(store.snapshot(CHANGE_ID).repository)
repo["phase"] = "post-build"
repo["revision"] = "fixture-r2"
for a in repo["artifacts"]:
    if a["ref"] == "code.place-order-handler":
        a["digest"] = "sha256:" + "9" * 64   # Builderが実装した結果
store.update_repository(repo)
```

以降の進行は次のとおり。

```text
state=needs-analysis             role=Analyst     [risk-signals-reviewed]
state=needs-analysis             role=Analyst     [affected-data-confirmed, operation-boundaries-confirmed]
state=needs-pre-build-challenge  role=Challenger  [design-challenged]
state=needs-evidence             role=Builder     [...]
state=needs-post-build-challenge role=Challenger  [...]
state=ready-to-merge
```

#### 何が問題か

- 同じ内容の確認をやり直すためだけに、Agentの作業が3回増える
- 実装前の設計反証が実装後に再実行される。Challengerには「まだ実装されていない前提で設計を反証せよ」という指示が、実装済みのコードに対して渡される
- 14.7 は`needs-post-build-analysis`という状態を定義しているが、実際に返るのは`needs-analysis`である。`risk-signals-reviewed`が実装前の工程に属するため、変更が実装前まで巻き戻る。文書と実装が食い違っている

#### テストで見えていない理由

実装後へ移るテストとgolden fixtureは、いずれも工程名と版番号だけを変え、コードのhashを1つも変えていない。

- `tests/test-vnext-prototype.py:904`
- `tests/test-vnext-prototype.py:1144`
- `prototype/vnext/golden/v1/application-lifecycle.json`の9番目の操作

つまり「Builderが何も書かなかった場合のbuild」だけを検証している。14.9 の7番、新しい候補がなければEvidenceへ進むという分岐は、現実の入力では通らない可能性が高い。

#### 対応の方向

候補の同一性を2つに分ける。

| 種別 | 内容 | 用途 |
|---|---|---|
| 再利用の識別子 | 検出処理の版、signal、対象を表す論理ID | 過去の確認結果を再利用してよいかを決める |
| 根拠の版 | 根拠となったコードの参照先とhash | 記録して説明に使う。識別子には含めない |

これにより、既存の書込み箇所を実装しただけでは再確認が発生せず、新しい書込み先や新しい接続先が現れたときだけAnalystへ戻る。13.8 が本来止めたかった条件はこちらである。

#### Rust版での対応

対応済み。

- candidate fingerprintをDetector ID・version、signal、bindingによる論理IDへ変更した
- 根拠ref・digestは`evidence_digest`として分離した
- `confirmed`は論理IDが同じなら再利用し、`not-applicable`は根拠digestが一致する場合だけ再利用する
- `ready-to-build`のContextへ実装前Resultを含め、Repository更新後の`result.build`が実装前Resultと実装後artifactを結ぶbuild baselineになるようにした
- post-buildの未確認candidateは`needs-post-build-analysis`を返すようにした
- lifecycle goldenで2つのコードartifact digestを実際に変更し、設計工程へ戻らず`needs-evidence`へ進むことを固定した

あわせて、実装後にコードが変わったことによる影響は、実装前の工程へ差し戻すのではなく実装後のChallengerとEvidenceへ渡す。

### 3.2 検証の優先順位が逆になっている

14.11 が未達成として挙げる項目の筆頭が「実コードからのsignal検出」である。これは 2.2 で述べたとおり、フレームワーク最大の設計問題である。

一方で、Release署名、鍵の失効と世代交代、tar展開の安全制限、再現可能なbuild、rollbackは実装済みで、Rust側の配布関連だけで1,500行を超える。これは利用者が存在して初めて必要になる部分であり、価値の仮説が一つの実リポジトリでも検証されていない段階で作る順番ではない。

配布まわりは、ローカルのReleaseとlock検証までで凍結する。

### 3.3 人を止める条件が広く、上限が設計されていない

13.5 と 15.7 が挙げる差し戻し条件は8項目あり、その多くは通常の書込み処理で成立する。たとえば「どの処理までを同じtransactionに含めるか複数の妥当な案がある」は、ほぼすべてのデータ更新で成立する。

5.7 は「未充足なら止める条件は、影響が大きいものに限定する」と書いているが、13章と15章はその限定を適用していない。8章に「不要な作業停止と人への確認回数」という指標はあるが、目標値も調整する手段も設計にない。7章の導入方式の選択は導入時の設定であり、変更ごとの強度を変える仕組みではない。

差し戻し条件そのものを適用Ruleで選べるようにし、既定では取り消せない操作と影響の大きいデータに限って止める形にする。そうしなければ、3.1 で挙げた「エージェントが規範を守らない」問題が「人が仕組みを迂回する」問題に置き換わる。

### 3.4 正本形式の記述と実装が食い違っていた

10章は正本形式を未決定としていた一方、14.18とPrototypeはMarkdown内のtyped blockを既に採用しており、設計文書内で状態が矛盾していた。

正本はHuman-first Markdown、機械判定の正本部分はSchema検証するtyped blockとする方針へ統一した。YAML Recordは移行中の互換入力であり、新規Recordの推奨形式にはしない。block外の本文は機械判定へ使わず、ContextにはRequirementに一致するtyped clause本文だけを投影する。詳細は2.6を参照。

### 3.5 Contract整備の負担は減っておらず、発生時点が移動しただけ

3.3 は「階層Contractを導入時に整備する負担が大きい」を解くべき問題として挙げ、7章の変更駆動の導入がその答えになっている。

しかし 13.4 の`data-contracts-ready`は、データを書き換える各操作について9項目の内容が条項として定まっていることを要求し、該当しない項目にも非該当の理由を求める。さらに「既存Contractから一意に決まらない項目は補完せず人へ戻す」と定めている。

Contractがほとんど無い状態で導入したリポジトリでは、最初にデータベースを触る変更で、9項目の条項作成と複数回の人の判断が一度に発生する。導入時の負担を変更時へ移しただけであり、予告なく作業の途中で発生する分、体感は悪化する可能性がある。

一つの案は、条項に「コードから復元しただけで承認されていない」という中間の状態を持たせ、変更が実際に依存する項目だけ人の承認を求める形である。ただし現在の設計では承認の有無が二値なので、これは意味構造の変更にあたる。

### 3.6 実行結果の記録をコードと同じPRに置く負荷

5.11.1 は実行結果の記録をGit管理とし、Action発行ごとに1ファイルとしている。14.9 の一つの変更で9件生成される。

これらは機械生成のJSONで、コードと同じPRに入る。レビュー担当者は毎回9件以上の生成ファイルを差分として見ることになり、実質的に読まずに通す運用へ流れる。また、参照したContractやコードが変わるたびに再提出と新規ファイル追加が起きるため、長く続くブランチではファイル数が線形以上に増える。

CIと別セッションから参照できるという要件は、通常のブランチとは別の参照先に置いても満たせる。現在の 5.11 は変更ブランチに直接置く形しか想定していないので、この選択肢を検討対象として明記する。

### 3.7 標準の確認事項を誰が作り続けるか

5.5.7 は「導入チームに確認事項とルールの継続保守を要求しない」と定め、フレームワーク側が持つとしている。方針としては正しいが、13章と15章の2領域だけで450行を使っている。認可、個人情報、外部API、並行処理、フロントエンドと広げると、少人数の保守者が支えられる規模を超える。

最初に配る標準セットの範囲を明示し、そこに含まれない領域はリポジトリ固有のルール追加で埋める前提にする。5.5.7 の昇格の流れ自体は良く設計されているので、リポジトリ固有のルール追加を例外ではなく通常経路として扱うだけで一貫する。

## 4. 妥当だと判断した設計

| 対象 | 評価 |
|---|---|
| 2章の問題定義と3.1 | 複数のSkillを順番に呼ばせる現行方式の弱点を正確に指摘している |
| 5.5.1 の責務分離 | 確認事項の定義、適用条件、検出処理、解決処理、配布単位を分けた判断は、従来案の結合を正しく解いている |
| 5.5.3 のルール合成 | 同じ確認事項IDに異なる定義があれば優先順位で隠さず設定エラーとして止める判断は、この種の仕組みが陥る暗黙の順序依存を最初から排除している |
| 14.8 の鮮度判定 | 変更全体を一つのlockで無効化せず、結論ごとに実際に読んだ参照先とhashを比べる設計は、現行のresolved lock方式より明確に優れている |
| 14.13 の説明機能 | 機械可読な形式を正本とし人向け表示をそこから生成するため、判定ロジックが二重にならない |
| 9章 | 解消できない問題を明示し、目標を「自動的に正解を決めること」に置いていない姿勢は正しい |

## 5. 対応の順序案

根本設計の層を先に置く。

1. 変更が2件あるfixtureと、`change_id`を持たないShared Contractを1件用意する。変更Aが確定した規範を変更Bが読み、人への再質問が発生しないことを検証する。同じ条項に触れる並行変更の意味論を決める。2.1
2. （Rust版の最初の縦断実装まで対応済み）コードからの物理観測と、symbol・resource単位のBinding Recordを分離する。Bindingにはownerと承認Decisionを持たせ、source、Binding、Decisionの変更を検出根拠の鮮度へ反映する。対応言語・frameworkを増やす際はcoverageをfail-closedのまま拡張する。2.2
3. （Rust版で対応済み）signalの`not-applicable`判定を候補ごとにChallengerへ確認させ、支持されなければ`confirmed`へ戻す。旧`excluded`値は廃止する。実在するsignalに対するRequirement省略は未実装のままfail closedとし、将来追加する場合も決定権限・適用範囲・期限を持つDecision経路として分離する。2.4
4. （Rust版で対応済み）各Requirementを「証拠で裏付けられる」と「形式しか検査できない」に分類する。標準Ruleでは実装検証の2件だけを`evidence-backed`とし、分析・設計・Challengeは`attestation`として保証範囲を明示する。CI／runner由来の保証は署名付きEvidenceを導入するまで対象外とする。2.3
5. 候補の同一性を、論理IDと根拠の版に分ける。実装後の差し戻し先を実装前の工程から実装後のChallengerへ変える。あわせて、実装でコードのhashが変わる場合をgolden fixtureとテストへ追加する。3.1
6. （Rust版で対応済み）Markdown正本のtyped clauseへ適用範囲を持たせ、Requirementに一致する条項本文だけをGenerated Contextへ投影する。2.6、3.4
7. （Rust版で対応済み）Signal CatalogでDetectorとRuleの語彙・bindingをcompile時に照合する。実装言語はRustへ一本化し、Pythonとの同等性を新規設計の受入条件から外す。配布まわりはローカルのReleaseとlock検証までで凍結する。2.7、3.2
8. （Rust版で対応済み）新規導入のContract・Decision既定rootをRepository直下へ戻し、`.agentic/`にはFrameworkの制御・進行管理Recordを置く。2.8
9. （指標と初期目標は確定、実測は未完了）同じ評価シナリオで現行方式とRust版の基準線を測り、実装開始後の仕様手戻り率を比較する。2.9
10. （Rust版の個別Changeゲートまで対応済み）Repository全体`ContractHealthReport`を基に、現在のChangeへ関係する`stale`・`failed`条項を`before-merge`で再検証する。残る定期CIの停止基準は、個別Changeとは別の運用ポリシーとして決める。2.5

## 6. 確定が必要な前提

- 現行Kitに実際の導入先があるか。無い、または管理下だけであれば、7.1 の互換層と段階移行は不要になり、相応の工数が浮く
- 現行のLevelとR0からR3の分類を新方式でどう扱うか。7.1 に記述がない
- 並行する変更どうしが同じ条項に触れる場合の意味論。片方を止めるのか、両方に再確認を要求するのか、Contractの正本をどう直列化するのか。2.1 の実装に先立って決める必要がある
