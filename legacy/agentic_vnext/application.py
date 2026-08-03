"""副作用を持つAdapterと、純粋なKernelを接続するApplication層。"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from .cache import EvaluationCache
from .context import ContextCompiler
from .detection import TypedFactDetector
from .explain import ExplainReport, ExplanationBuilder
from .framework_lock import FrameworkLock, validate_framework_lock
from .kernel import ThinKernel
from .model import GeneratedContext, KernelDecision
from .project import ProjectStore
from .rules import compile_rule_index
from .submission import prepare_result


@dataclass(frozen=True)
class NextResponse:
    decision: KernelDecision
    context: GeneratedContext | None
    cache_diagnostics: tuple[str, ...] = ()


class Application:
    """`next`と`submit`を提供し、Moduleの実行順だけを管理する。

    意味上の判定はKernelへ、保存はProject Storeへ委譲する。ここへ業務Ruleを
    書かないことで、将来CLI・CI・HTTPから呼んでも同じ判定経路を利用できる。
    """

    def __init__(
        self,
        store: ProjectStore,
        rule_source: dict[str, Any],
        framework_lock: dict[str, Any],
        cache: EvaluationCache | None = None,
    ):
        self.store = store
        self.rule_index = compile_rule_index(rule_source)
        # Validate before constructing any evaluator so a partially upgraded
        # Framework cannot produce a decision.
        self.framework_lock: FrameworkLock = validate_framework_lock(
            framework_lock,
            rule_source,
            self.rule_index,
        )
        self.cache = cache
        self.detector = TypedFactDetector()
        self.kernel = ThinKernel()
        self.context_compiler = ContextCompiler()
        self.explanation_builder = ExplanationBuilder()
        # PrototypeではActionの再送・改ざんを検出するためメモリに保持する。
        # 本実装では永続Action Recordまたは署名済みtokenへ置き換える。
        self._issued: dict[str, GeneratedContext] = {}

    def next(self, change_id: str) -> NextResponse:
        """現在の正本から次の一手を毎回再計算する。State自体は保存しない。"""

        snapshot = self.store.snapshot(change_id)
        detection = self.detector.detect(snapshot)
        decision = self.kernel.evaluate(snapshot, self.rule_index, detection)
        context = self.context_compiler.compile(decision, snapshot, detection)
        cache_diagnostics: tuple[str, ...] = ()
        if self.cache is not None:
            try:
                self.cache.write_evaluation(
                    self.framework_lock.digest,
                    snapshot,
                    self.rule_index,
                    detection,
                    decision,
                )
            except OSError as error:
                # A disposable cache must never become a workflow gate.
                cache_diagnostics = (f"cache write failed: {error}",)
        if context is not None:
            self._issued[context.action_id] = context
        return NextResponse(
            decision=decision,
            context=context,
            cache_diagnostics=cache_diagnostics,
        )

    def explain(self, change_id: str) -> ExplainReport:
        """Recompute and explain the current decision without issuing an Action."""

        snapshot = self.store.snapshot(change_id)
        detection = self.detector.detect(snapshot)
        decision = self.kernel.evaluate(snapshot, self.rule_index, detection)
        return self.explanation_builder.build(
            snapshot,
            self.rule_index,
            detection,
            decision,
        )

    def submit(
        self,
        change_id: str,
        action_id: str,
        context_digest: str,
        role: str,
        result_schema: str,
        payload: dict[str, Any],
        output_refs: tuple[str, ...] = (),
    ) -> NextResponse:
        """発行済みActionに対するResultを検証・保存し、次状態を返す。"""

        context = self._issued.get(action_id)
        if context is None:
            raise ValueError(f"unknown action: {action_id}")
        current = self.store.snapshot(change_id)
        result = prepare_result(
            context=context,
            current=current,
            change_id=change_id,
            action_id=action_id,
            context_digest=context_digest,
            role=role,
            result_schema=result_schema,
            payload=payload,
            output_refs=output_refs,
        )
        self.store.append_result(result)
        del self._issued[action_id]
        return self.next(change_id)
