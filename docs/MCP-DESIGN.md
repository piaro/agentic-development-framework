# MCP Adapter 実装設計

## 1. Status

この文書は、Rust PrototypeへAgent用の書込み経路を接続するための実装設計です。
MCPを通常のAgent経路、CLIをCI・診断・配布・復旧用の補助経路とします。

local stdio server、発行済みAction管理、Result／Evidence／Decision／Contract書込みToolは
Rust版へ実装済みです。本書は引き続きTool契約と信頼境界の正本です。

## 2. 結論

- 同じRust binaryへlocal stdio MCP serverを追加する。
- MCP serverは起動時に一つのProject rootへ固定し、Tool引数から任意pathを受け取らない。
- `next`が発行したActionとGenerated ContextはMCP session内で保持する。
- Agentの作業結果は必ずApplicationの`submit`境界を通し、Resultを直接生成させない。
- Evidence、Decision、Contractの書込みも、発行済みActionに結び付けた専用Toolから行う。
- MCPとCLIにKernel、Store、Result生成順序を重複実装しない。
- 最初の実装はstdioだけを対象とし、remote HTTP MCPは認証・利用者分離を設計するまで追加しない。

## 3. 解決する問題

現在のCLIは`next`でActionを表示できますが、Agentが次の操作へ到達できません。

- Action Resultの提出
- Evidenceの追記
- Human回答を受けたDecisionの記録
- Decisionを反映したContractの更新

ライブラリには対応するApplication／Project Store APIがありますが、Agent用Adapterから呼び出せません。
そのため、完全なlifecycleはgolden testでは通っても、実Projectの利用者経路では最初のActionから進行できません。

## 4. 目標と非目標

### 4.1 目標

- Agentが構造化Toolとして`next`と`submit`を利用できる。
- Tool入力と出力をJSON Schemaで固定する。
- Action発行時のContextと提出時のProject Snapshotを照合する。
- 同一MCP session中のコード・Contract・Decision・Evidence変更を正しく`output_refs`として扱う。
- server再起動、二重提出、競合、stale Actionをfail closedにする。
- text表示とMCP結果が同じApplication判断を使用する。

### 4.2 非目標

- MCP serverが分析、設計、Human判断を自動決定すること
- remote MCP serverの公開
- Agent会話全文やGenerated Context全文のGit保存
- 複数Projectを一つのserver processから操作すること
- Result／Evidenceを信頼できるCIが生成したことの署名検証
- 複数fileを跨ぐ汎用transaction engine

## 5. 全体構成

```text
Agent / MCP Host
       │ MCP over stdio
       ▼
adf mcp --project <root>
       │
       ▼
McpProjectSession
  ├─ IssuedActionRegistry
  ├─ ProjectApplicationService
  └─ MCP Tool Router
       │
       ├─ LoadedProject / Git Repository Adapter
       ├─ Application
       └─ Filesystem Project Store
              │
              ├─ Change / Result / Evidence
              ├─ Contract
              └─ Decision
```

`McpProjectSession`は接続ごとの操作状態を持ちますが、StateやNext Actionの正本にはなりません。
各Tool callは現在のGit・Record・Framework lockを読み直します。

## 6. Binaryとtransport

追加する起動形式は次のとおりです。

```sh
adf mcp \
  --project /path/to/project \
  [--release /path/to/offline-release]
```

- transportはstdioだけを実装する。
- stdoutはMCPのJSON-RPC message専用とする。
- logと診断はstderrへ出す。
- server起動時にProject rootをcanonicalizeし、以後変更しない。
- ReleaseはFramework lockから通常どおり検証する。`--release`も既存CLIと同じ検証を迂回できない。
- 一つのMCP serverは一つのProjectだけを扱う。別Projectには別processを起動する。
- Rust実装は公式MCP Rust SDKである`rmcp`を第一候補とし、Cargo.lockでversionを固定する。

MCPはstateful sessionですが、session IDだけを認可根拠にはしません。最初の実装はlocal
stdio processに限定し、MCP Hostと同じlocal principalを信頼境界とします。

## 7. Application serviceの再構成

長時間動作するMCP serverで`LoadedProject`や`Application`を作り置きすると、Agentが変更した
Git内容を観測できません。一方、Tool callごとにApplicationを破棄すると発行済みContextを失います。

このため、次の二つを分離します。

### 7.1 `ProjectApplicationService`

Tool callごとに現在のProjectを読み直し、既存Application処理を呼びます。

```text
evaluate_next(change_id) -> NextResponse + IssuedAction
submit_issued(issued_action, action_result) -> SubmitResponse
add_action_output(issued_action, record) -> OutputRef
```

`submit_issued`は次の順序をApplication内で一度だけ実装します。

1. 現在のProjectとFramework Releaseを再読込みする
2. 発行済みContextと提出されたAction ID・Context digestを照合する
3. 現在のSnapshotを生成する
4. `output_refs`以外の発行時入力がstaleでないことを検査する
5. Result payload、Role、Result Schema、候補、Requirement、根拠参照を検査する
6. 内容由来Result IDを生成する
7. Resultを排他的に追記する
8. 最新ProjectからKernelを再評価する

MCP handlerが`prepare_result`やStore書込み順を再実装してはいけません。

### 7.2 `IssuedActionRegistry`

発行済みActionは`action_id`だけでなく、次の組で識別します。

```text
(change_id, action_id, context_digest)
```

同じAction IDが異なる入力版で再利用されるため、`action_id`だけをkeyにしてはいけません。

Registry entryは次を保持します。

```yaml
protocol_version: "1"
change_id: change.example
action_id: action.example
context_digest: sha256:...
framework_lock_digest: sha256:...
rule_index_digest: sha256:...
issued_context: {}
allowed_output_kinds: []
registered_output_refs: []
```

- RegistryはMCP sessionのmemoryにだけ保持する。memoryにないActionは、正本の再評価と一致するときに再構成する。再構成後に`adf_next`を呼んだ場合も、同じChangeの正本に保存済みのEvidence、Decision、Contractは提出時の出力として参照できる。
- Generated ContextをGit、Result、derived cacheへ保存しない。
- Evidence、Decision、Contractの専用Toolが保存したrefだけを`registered_output_refs`へ追加する。
- 正常な`submit`後にexact keyを消費し、再評価で返した次Actionを新しいentryとして登録する。
- submit失敗時は、修正して再試行できるようentryを残す。
- 同じkeyへ複数提出が競合した場合、Filesystem Storeのexclusive createを最終防衛線とする。

## 8. server再起動時の扱い

Action IDはAction本体のdigest、Context digestはその生成元入力のdigestです。したがって、入力が動いていなければ、再評価は同じAction IDとContext digestを再生成します。この決定性がAction受理の根拠であり、発行済みActionをどこかへ保存する必要はありません。

再起動やMCP接続断のあとに未提出Actionを提出した場合、serverは現在の正本から再評価します。

- 再評価が同じActionを返すなら、そのActionは今も現在のものなので受理する。再起動前に行った作業はやり直さない。
- 再評価が別のActionを返すなら、`ACTION_NOT_CURRENT`で拒否し、現在のAction IDとContext digestを示す。Agentは`adf_next`から現在のActionに対してやり直す。
- 同じActionとContextに対するResultが既にあるなら、二重提出として冪等に再生し、Resultを二重に書かない。内容が違えば`WRITE_CONFLICT`にする。
- 既に書いたContract、Decision、Evidence、コードは削除やrollbackをせず、現在入力として再評価する。

Generated Contextを正本化しない方針は変えません。受理の根拠はmemoryではなく、正本を再評価した結果との一致です。derived cacheのreadをAction認証へ流用してはいけません。

再起動を跨いだActionでは、そのActionがどのRecordを書いたかというsession記憶が失われます。再接続後に`adf_next`を呼ぶと新しいsession entryが作られますが、`output_refs`のRecord参照は、そのentryで記録した出力またはChangeの正本に存在するRecordで検査します。これにより、接続断前に保存したEvidenceを、正本の再評価で同じActionが返る場合に提出できます。

## 9. MCP Tool一覧

Tool名は広いMCP client互換性を優先し、ASCII英数字とunderscoreだけを使用します。

| Tool | 種別 | 役割 |
|---|---|---|
| `adf_next` | read | 現在State、Next Action、Contextを発行する |
| `adf_explain` | read | 現在の判定理由を説明する |
| `adf_contract_health` | read | Repository全体のContract healthを表示する |
| `adf_execution_log` | read | 保存済みResultと実行RecordからContextサイズと計測値を集計する |
| `adf_begin_execution` | write | 現在のActionに対する外部実行の開始を追記する。Agentは起動しない |
| `adf_complete_execution` | write | 外部実行の成否と確定済みの利用量を追記する |
| `adf_submit` | write | 発行済みActionのResultを検証・保存し、再評価する |
| `adf_add_evidence` | write | 発行済みEvidence ActionへEvidenceを追記する |
| `adf_apply_decision` | write | Human回答を解決するDecisionを保存する |
| `adf_apply_contract` | write | Decisionを反映したContractを楽観的lock付きで更新する |
| `adf_abandon_action` | local state | 未提出Actionをsession内で明示的に破棄する |

`adf_contract_health`は診断用Reportを返し、CIの成否を決めません。Repository全体を停止する運用policyはproject所有fileとしてGit管理し、CLIの`contract-health --policy`だけがprocess終了codeへ反映します。

MCP Tool annotationはHost向けhintとして設定しますが、認可には使用しません。

- read Tool: `readOnlyHint: true`
- write Tool: `readOnlyHint: false`
- Contract更新: `destructiveHint: true`
- append-only Result／Evidence: `destructiveHint: false`

## 10. Tool契約

全Toolは`inputSchema`と`outputSchema`を公開し、成功時は`structuredContent`を返します。
保存Record Schemaとは別に、MCP I/O Schemaを`schemas/mcp/v1/`へ置きます。

### 10.1 `adf_next`

Input:

```json
{
  "change_id": "change.example",
  "require_clean": false
}
```

Output:

```json
{
  "schema_version": "1",
  "next_response": {},
  "issued_action": {
    "change_id": "change.example",
    "action_id": "action.example",
    "context_digest": "sha256:..."
  }
}
```

- `issued_action`はActionがない場合`null`とする。
- AgentへRegistry内部値や秘密値を返さない。
- `next_response`は既存Next Response v1を再利用する。

### 10.2 `adf_submit`

Input:

```json
{
  "change_id": "change.example",
  "action_id": "action.example",
  "context_digest": "sha256:...",
  "payload": {},
  "execution": {
    "duration_ms": 1200,
    "model": "example-model",
    "input_tokens": 900,
    "output_tokens": 180
  },
  "output_refs": []
}
```

RoleとResult Schemaは発行済みActionから導出します。Agent入力として受け取りません。

Output:

```json
{
  "schema_version": "1",
  "result_id": "result.example",
  "already_completed": false,
  "next_response": {}
}
```

- exact action keyがRegistryにない提出は、正本を再評価して同じActionが現在のものであるときだけ受理する。
- 成功したResult追記後だけActionを消費する。
- Contract、Decision、Evidenceの`output_refs`は、同じentryの`registered_output_refs`に存在しなければ拒否する。再起動を跨いだActionでは、そのRecordがChangeの正本に存在することで代える。
- Repository artifactの`output_refs`は`implement-change` Actionだけに許可する。
- 同じ内容のtransport retryは、Registry消費後でも既存Resultと提出内容が一致すれば成功として返せるようにする。
- 同じAction／Contextに異なる内容が既にあれば競合として拒否する。
- `execution`は実行環境がすでに持つ値だけを任意で受け取る。ADFはContextサイズを提出検証中に計算し、計測のためのLLM実行、タイマー、追跡処理を追加しない。
- `execution`はResult ID、freshness、Kernel判断に影響しない。不明な値は推測せず省略する。

### 10.2.1 外部実行Record

外部RunnerはAction実行前に`adf_begin_execution`を呼び、Change ID、Action ID、Context digestを現在のActionと照合します。ADFはContextの直列化サイズ、Role、Result Schemaを自分で記録します。Runnerの申告値で置き換えません。

実行後は`adf_complete_execution`で成否、Result ID、Token数、処理時間、実行環境が報告した費用を追記します。Token数は通常入力、キャッシュ作成、キャッシュ読取、出力、推論を区別します。成功には同じActionとContextから作られたResult IDが必要です。失敗、中断、staleはResultなしで記録できます。開始と完了は別fileへ排他的に追記し、完了の再送は内容が同じ場合だけ冪等に受理します。不明な利用量や費用は推測せず省略します。

実行RecordはKernelへ渡しません。Action選択、Result ID、freshness、Evidenceの充足判定には影響しません。Agent会話、Generated Context全文、CodexのJSONL、Claude Codeの完全なJSON応答も保存しません。

### 10.3 `adf_add_evidence`

Input:

```json
{
  "change_id": "change.example",
  "action_id": "action.example",
  "context_digest": "sha256:...",
  "evidence": {}
}
```

- 発行Actionのexpected Result Schemaが`result.evidence`でなければ拒否する。
- Evidenceの`change_id`がActionと一致しなければ拒否する。
- Evidenceの`requirement_instances`が発行ActionのInstance集合を逸脱した場合は拒否する。
- 同じEvidence IDは上書きしない。
- 成功時にEvidence IDを返し、Agentは後続`adf_submit.output_refs`と`basis_refs`へ含める。

### 10.4 `adf_apply_decision`

- 発行Actionが`record-human-decision`または`establish-impact-governance`でなければ拒否する。
- Decisionの`change_id`がActionと一致しなければ拒否する。
- `resolves`がContext内の回答済みDecision Requestを含まなければ拒否する。
- accepted Decisionは対応するHuman Resultが現在Snapshotに存在する場合だけ受理する。
- Contractと同様に`expected_digest`を必須とし、既存Decision更新は楽観的lock付きでatomic replaceする。
- 新規Decisionは`expected_digest: null`を明示し、同じIDが既に存在すれば拒否する。

### 10.5 `adf_apply_contract`

InputにはContract Recordと`expected_digest`を必須とし、条項単位で更新する場合は
`expected_digest: null`と`expected_clause_digests`を渡します。後者は
`{条項ID: 読取り時の条項digest}`のmappingです。

- 発行Actionが`record-human-decision`でなければ拒否する。
- `record-human-decision`では、Contract変更が同じActionで記録したDecisionまたはContext内の既存Decisionをauthorityとして参照することを検査する。`establish-impact-governance`では、エージェントが現在のChangeに明示された要求で決まる最小限のContractだけを作る。内容が最小限かどうかは機械判定しない。
- `expected_digest`と`expected_clause_digests`の同時指定を拒否する。
- 全体更新では`expected_digest`省略、現在digestとの不一致を拒否する。
- 条項更新では`contract.clauses`をpatchとして扱う。変更する既存条項はpayloadとdigestの両方へ含め、削除する条項はdigestだけへ含める。対象外の条項はどちらにも含めない。
- 対象条項が先に変更されていればstaleとして拒否する。対象外の条項が変更されていても最新版を保持して機械的に併合する。Contract metadataは併合せず、全体更新を要求する。
- 新規Contractは`expected_digest: null`を明示し、同じIDが既に存在すれば拒否する。
- Agentが任意のShared Contractを汎用編集するToolにはしない。

### 10.6 `adf_abandon_action`

- session memoryからexact action keyだけを削除する。
- Project Recordやコードを変更しない。
- 既に書かれたEvidence、Decision、Contractを自動削除しない。

## 11. Actionごとの許可書込み

書込みToolは、MCP HostのTool公開だけに依存せず、server側で発行Actionと照合します。

| Action | 許可する事前書込み |
|---|---|
| `assess-change-impact` | なし |
| `establish-impact-governance` | Contract |
| `review-risk-signals` | なし |
| `analyze-requirements` | なし |
| `answer-decision-request` | なし。回答はResult payloadとして提出 |
| `record-human-decision` | Decision、Contract |
| `challenge-result` | なし |
| `implement-change` | Repository file変更。MCP外のworkspace操作 |
| `record-evidence` | Evidence |

この対応はMCP handlerへ散在させず、Application protocolの固定tableとして一か所に置きます。
未知Actionは書込みなしとしてfail closedに扱います。

## 12. 代表フロー

### 12.1 Analyst

```text
adf_next
  → review-risk-signals Actionを発行
AgentがコードとContextを確認
adf_submit
  → Result検証・追記
  → 次のActionを返す
```

### 12.2 Human Authority

```text
adf_next
  → Human Action
MCP HostがHumanへ質問
Human回答をAgentがadf_submit
  → record-human-decisionを返してsessionへ登録
adf_apply_decision
adf_apply_contract
adf_submit(output_refs = [Decision, Contract])
```

HumanはJSONやContractを直接編集せず、回答だけを行います。

### 12.3 BuildとEvidence

```text
adf_next
  → implement-change
Agentがworkspaceのコードを変更
adf_submit(output_refs = changed artifact refs)
  → needs-evidence Actionを返してsessionへ登録
adf_add_evidence
adf_submit(
  output_refs = [evidence ref],
  payload.outcomes[].basis_refs = [evidence ref, ...]
)
```

MCP serverは各callでGit Adapterを再実行するため、Agentによるコード変更後のdigestを観測します。
発行時Contextはsession Registryから取得し、変更したrefだけを`output_refs`として許可します。

## 13. Error model

JSON-RPC自体の不正はprotocol error、Tool実行中の業務エラーは`isError: true`のTool resultとします。
Tool errorは機械可読な次の共通形式を返します。

```json
{
  "schema_version": "1",
  "code": "CONTEXT_STALE",
  "message": "issued context is stale: contract.example",
  "retryable": false,
  "details": {}
}
```

固定する主要code:

- `INVALID_ARGUMENT`
- `PROJECT_INVALID`
- `ACTION_NOT_CURRENT`
- `ACTION_NOT_ALLOWED`
- `CONTEXT_STALE`
- `OUTPUT_REF_INVALID`
- `ALREADY_COMPLETED`
- `WRITE_CONFLICT`
- `RELEASE_MISMATCH`
- `INTERNAL`

Agentが入力を修正できるSchema・domain errorと、server再起動が必要な内部errorを区別します。

## 14. Concurrencyとretry

- write Toolは`(change_id, action_id, context_digest)`単位でsession内mutexを取得する。
- DecisionとContract全体の更新は`expected_digest`を要求する。Contract条項更新は`expected_clause_digests`を要求し、いずれも現在値をlock内で再読込みする。
- 同じContractでも別条項の並行更新は最新版を保持してatomicに併合し、同じ条項の並行更新はstaleとして拒否する。
- ResultとEvidenceはexclusive createを維持する。
- 同一内容のretryは成功済み結果を返し、異なる内容の二重提出は拒否する。
- 複数MCP process間の競合はFilesystem Storeのlockとexclusive createで解決する。
- write完了前にMCP接続が切れた場合、再接続後はRecordを読み、同一内容が保存済みか確認してから再試行する。

## 15. Security境界

- v1はlocal stdioだけを許可する。
- serverのProject rootを起動引数で固定し、Toolからpathを受け取らない。
- symlink、Repository外path、`.adf/cache`を正本rootに指定することを既存Storeと同様に拒否する。
- Tool annotationを認可根拠にしない。
- write Toolは発行Action、Role、Result Schema、許可output kindをserver側で照合する。
- secret、Agent会話、Generated Context全文をlogへ出さない。
- stderr logにはAction ID、Context digestの短縮値、Result ID、error codeだけを記録する。
- remote transportを追加する場合は、認証、利用者identity、Project authorization、CSRF／DNS rebinding対策を別Contractとして先に定義する。

## 16. CLIとの関係

MCP実装後も、既存CLIを削除しません。

- `next`、`explain`、policyなしの`contract-health`: 人、診断用
- `contract-health --policy`: Repository全体の定期CIゲート
- `release`、`binary`、`verify-*`: 配布・互換性検査用
- `mcp`: Agentの通常経路
- `execution begin/complete`: 外部RunnerがAction実行の前後に使う操作Record経路

Result、Evidence、Decision、Contractのwrite CLIはMCP v1の必須範囲に含めません。one-shot CLIでは発行時Contextをprocess間で安全に渡す追加protocolが必要だからです。実行Recordだけは例外です。`execution begin`が現在のActionを再評価して完全一致を確認し、`execution complete`は保存済みの開始Recordへだけ追記するため、Result生成処理をCLIへ複製しません。

将来write CLIを追加する場合は、MCP Toolと同じI/O Schemaと
`ProjectApplicationService`を利用し、独自のResult生成処理を持たせません。

## 17. 実装単位

想定する主な変更は次のとおりです。

```text
src/
├── application.rs             # submit_issuedとAction output検証
├── project_application.rs     # Tool callごとのProject再読込み
├── mcp_server.rs              # RMCP Tool routerとstdio transport
└── main.rs                    # mcp subcommand

schemas/mcp/v1/
├── next-input.schema.json
├── next-output.schema.json
├── submit-input.schema.json
├── submit-output.schema.json
└── tool-error.schema.json
```

RMCP、Tokio、schema生成用crateを追加する場合も、公開Schemaの正本はRepository上のJSON
Schemaとし、生成差分をtestで検査します。

## 18. Test方針

### 18.1 Unit

- Tool入力・出力Schema
- exact action key
- Actionごとの書込み許可table
- Tool error mapping
- same-content retryとdifferent-content conflict

### 18.2 Application integration

- Tool callごとのProject再読込み
- 発行後のコード変更を`output_refs`付きでsubmit
- 発行後の許可されていない入力変更をstaleとして拒否
- 専用Toolを経由していないContract、Decision、Evidenceの`output_refs`を拒否
- Decision、Contract、EvidenceのAction binding
- submit失敗時にIssued Actionを消費しない
- 成功時にexact Actionだけを消費し、返した次Actionを登録する

### 18.3 MCP protocol integration

実binaryをstdio subprocessとして起動し、MCP clientから次を検証します。

1. initialize
2. tools/list
3. `adf_next`
4. `adf_submit`
5. 再評価後の次Action
6. lifecycle全体を`ready-to-merge`まで実行
7. stdoutへJSON-RPC以外を出さない
8. server停止時に未提出Actionを失効

### 18.4 Acceptance criteria

- 実MCP Toolだけで初期`needs-analysis`から`ready-to-merge`へ到達できる。
- 12操作lifecycleの各checkpointが既存goldenと一致する。
- MCP経路と既存Application goldenが同じResult ID、State、Action ID、Context digestを返す。
- MCP handlerからStoreや`prepare_result`を直接呼ぶ重複実装がない。
- Result、Evidence、Decision、Contractの手動上書きを必要としない。
- server再起動後にmemoryだけを根拠としてActionを受理しない。受理の根拠は正本を再評価した結果との一致とする。

## 19. 実装順

1. `Application`のissued keyをAction ID＋Context digestへ変更する。
2. 現在Projectを毎回読み直す`ProjectApplicationService`を追加する。
3. `adf_next`、`adf_submit`だけでrisk reviewを一段進める。
4. MCP subprocess integration testを追加する。
5. Human回答、Decision、Contract Toolを追加する。
6. Evidence Toolとbuild後flowを追加する。
7. MCPだけで完全lifecycleを通す。
8. 再起動を跨いだActionの提出を、正本の再評価との一致で受理する。

最初のmilestoneは「Tool一覧が存在する」ことではなく、実MCP clientから
`next → submit → 次のnext`が永続Recordを介して成立することとします。

## 20. 参照仕様

- Model Context Protocol Specification 2025-11-25: Tools
  - <https://modelcontextprotocol.io/specification/2025-11-25/server/tools>
- Model Context Protocol Specification 2025-11-25: Lifecycle
  - <https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle>
- Model Context Protocol Specification 2025-11-25: Transports
  - <https://modelcontextprotocol.io/specification/2025-11-25/basic/transports>
- Official Rust SDK `rmcp`
  - <https://github.com/modelcontextprotocol/rust-sdk>
