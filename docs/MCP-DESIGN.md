# vNext MCP Adapter 実装設計

## 1. Status

この文書は、Rust vNext PrototypeへAgent用の書込み経路を接続するための実装設計です。
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
agentic mcp --project <root>
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
agentic mcp \
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

- RegistryはMCP sessionのmemoryにだけ保持する。
- Generated ContextをGit、Result、derived cacheへ保存しない。
- Evidence、Decision、Contractの専用Toolが保存したrefだけを`registered_output_refs`へ追加する。
- 正常な`submit`後にexact keyを消費し、再評価で返した次Actionを新しいentryとして登録する。
- submit失敗時は、修正して再試行できるようentryを残す。
- 同じkeyへ複数提出が競合した場合、Filesystem Storeのexclusive createを最終防衛線とする。

## 8. server再起動時の扱い

v1では、MCP processの終了時に未提出Actionを失効させます。

再接続後は`agentic_next`を再実行し、現在の正本からActionを再発行します。

- 入力が変わっていなければ、同じAction IDとContext digestを再生成できる。
- 入力が変わっていれば、古いActionを暗黙に復元せず、現在のActionからやり直す。
- Action発行後にコードやRecordを変更し、その途中でserverが停止した場合は、v1では自動resumeしない。
- 既に書いたContract、Decision、Evidence、コードは削除やrollbackをせず、現在入力として再評価する。

再起動を跨ぐresumeが必要になった場合は、Generated Contextを正本化せず、MCP Hostが保持する
署名付きAction receipt、または`.agentic/tmp/`のlocal capabilityとして別versionで設計します。
derived cacheのreadをAction認証へ流用してはいけません。

## 9. MCP Tool一覧

Tool名は広いMCP client互換性を優先し、ASCII英数字とunderscoreだけを使用します。

| Tool | 種別 | 役割 |
|---|---|---|
| `agentic_next` | read | 現在State、Next Action、Contextを発行する |
| `agentic_explain` | read | 現在の判定理由を説明する |
| `agentic_contract_health` | read | Repository全体のContract healthを表示する |
| `agentic_submit` | write | 発行済みActionのResultを検証・保存し、再評価する |
| `agentic_add_evidence` | write | 発行済みEvidence ActionへEvidenceを追記する |
| `agentic_apply_decision` | write | Human回答を解決するDecisionを保存する |
| `agentic_apply_contract` | write | Decisionを反映したContractを楽観的lock付きで更新する |
| `agentic_abandon_action` | local state | 未提出Actionをsession内で明示的に破棄する |

`agentic_contract_health`は診断用Reportを返し、CIの成否を決めません。Repository全体を停止する運用policyはproject所有fileとしてGit管理し、CLIの`contract-health --policy`だけがprocess終了codeへ反映します。

MCP Tool annotationはHost向けhintとして設定しますが、認可には使用しません。

- read Tool: `readOnlyHint: true`
- write Tool: `readOnlyHint: false`
- Contract更新: `destructiveHint: true`
- append-only Result／Evidence: `destructiveHint: false`

## 10. Tool契約

全Toolは`inputSchema`と`outputSchema`を公開し、成功時は`structuredContent`を返します。
保存Record Schemaとは別に、MCP I/O Schemaを`schemas/mcp/v1/`へ置きます。

### 10.1 `agentic_next`

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

### 10.2 `agentic_submit`

Input:

```json
{
  "change_id": "change.example",
  "action_id": "action.example",
  "context_digest": "sha256:...",
  "payload": {},
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

- exact action keyがRegistryにない提出を拒否する。
- 成功したResult追記後だけActionを消費する。
- Contract、Decision、Evidenceの`output_refs`は、同じentryの`registered_output_refs`に存在しなければ拒否する。
- Repository artifactの`output_refs`は`implement-change` Actionだけに許可する。
- 同じ内容のtransport retryは、Registry消費後でも既存Resultと提出内容が一致すれば成功として返せるようにする。
- 同じAction／Contextに異なる内容が既にあれば競合として拒否する。

### 10.3 `agentic_add_evidence`

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
- 成功時にEvidence IDを返し、Agentは後続`agentic_submit.output_refs`と`basis_refs`へ含める。

### 10.4 `agentic_apply_decision`

- 発行Actionが`record-human-decision`でなければ拒否する。
- Decisionの`change_id`がActionと一致しなければ拒否する。
- `resolves`がContext内の回答済みDecision Requestを含まなければ拒否する。
- accepted Decisionは対応するHuman Resultが現在Snapshotに存在する場合だけ受理する。
- Contractと同様に`expected_digest`を必須とし、既存Decision更新は楽観的lock付きでatomic replaceする。
- 新規Decisionは`expected_digest: null`を明示し、同じIDが既に存在すれば拒否する。

### 10.5 `agentic_apply_contract`

InputにはContract Recordと`expected_digest`を必須とし、条項単位で更新する場合は
`expected_digest: null`と`expected_clause_digests`を渡します。後者は
`{条項ID: 読取り時の条項digest}`のmappingです。

- 発行Actionが`record-human-decision`でなければ拒否する。
- Contract変更が、同じActionで記録したDecisionまたはContext内の既存Decisionをauthorityとして参照することを検査する。
- `expected_digest`と`expected_clause_digests`の同時指定を拒否する。
- 全体更新では`expected_digest`省略、現在digestとの不一致を拒否する。
- 条項更新では`contract.clauses`をpatchとして扱う。変更する既存条項はpayloadとdigestの両方へ含め、削除する条項はdigestだけへ含める。対象外の条項はどちらにも含めない。
- 対象条項が先に変更されていればstaleとして拒否する。対象外の条項が変更されていても最新版を保持して機械的に併合する。Contract metadataは併合せず、全体更新を要求する。
- 新規Contractは`expected_digest: null`を明示し、同じIDが既に存在すれば拒否する。
- Agentが任意のShared Contractを汎用編集するToolにはしない。

### 10.6 `agentic_abandon_action`

- session memoryからexact action keyだけを削除する。
- Project Recordやコードを変更しない。
- 既に書かれたEvidence、Decision、Contractを自動削除しない。

## 11. Actionごとの許可書込み

書込みToolは、MCP HostのTool公開だけに依存せず、server側で発行Actionと照合します。

| Action | 許可する事前書込み |
|---|---|
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
agentic_next
  → review-risk-signals Actionを発行
AgentがコードとContextを確認
agentic_submit
  → Result検証・追記
  → 次のActionを返す
```

### 12.2 Human Authority

```text
agentic_next
  → Human Action
MCP HostがHumanへ質問
Human回答をAgentがagentic_submit
  → record-human-decisionを返してsessionへ登録
agentic_apply_decision
agentic_apply_contract
agentic_submit(output_refs = [Decision, Contract])
```

HumanはJSONやContractを直接編集せず、回答だけを行います。

### 12.3 BuildとEvidence

```text
agentic_next
  → implement-change
Agentがworkspaceのコードを変更
agentic_submit(output_refs = changed artifact refs)
  → needs-evidence Actionを返してsessionへ登録
agentic_add_evidence
agentic_submit(
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
- `ACTION_NOT_ISSUED`
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
- symlink、Repository外path、`.agentic/cache`を正本rootに指定することを既存Storeと同様に拒否する。
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

write CLIはMCP v1の必須範囲に含めません。one-shot CLIでは発行時Contextをprocess間で安全に渡す追加protocolが必要だからです。

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
3. `agentic_next`
4. `agentic_submit`
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
- server再起動後にmemoryだけを根拠として古いActionを受理しない。

## 19. 実装順

1. `Application`のissued keyをAction ID＋Context digestへ変更する。
2. 現在Projectを毎回読み直す`ProjectApplicationService`を追加する。
3. `agentic_next`、`agentic_submit`だけでrisk reviewを一段進める。
4. MCP subprocess integration testを追加する。
5. Human回答、Decision、Contract Toolを追加する。
6. Evidence Toolとbuild後flowを追加する。
7. MCPだけで完全lifecycleを通す。
8. 使用実績を見て、再起動を跨ぐAction receiptとwrite CLIの要否を判断する。

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
