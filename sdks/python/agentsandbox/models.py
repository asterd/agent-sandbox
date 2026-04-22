"""Value objects exchanged between the SDK and the daemon.

The daemon contract lives in ``docs/api-http-v1.md``. These classes are thin
mirrors of that contract; keep them in sync when the API evolves.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, ClassVar


@dataclass(frozen=True, slots=True)
class ExecResult:
    """Outcome of a single ``Sandbox.exec`` call."""

    stdout: str
    stderr: str
    exit_code: int
    duration_ms: int

    @property
    def success(self) -> bool:
        return self.exit_code == 0

    def __str__(self) -> str:
        return self.stdout


@dataclass(frozen=True, slots=True)
class ExecStreamEvent:
    """Single event produced by ``Sandbox.exec_stream``."""

    event: str
    chunk: str | None = None
    exit_code: int | None = None
    duration_ms: int | None = None
    sandbox_id: str | None = None
    backend: str | None = None


@dataclass(frozen=True, slots=True)
class SandboxInfo:
    """Inspect response for a sandbox.

    Mirrors the daemon's ``InspectResponse`` (see
    ``crates/agentsandbox-daemon/src/handlers.rs``): ``created_at`` and
    ``expires_at`` are ISO-8601 strings as returned by the daemon; we keep
    them as strings to avoid timezone surprises at the SDK boundary.
    """

    sandbox_id: str
    status: str
    backend: str
    created_at: str
    expires_at: str
    error_message: str | None = None


@dataclass(slots=True)
class SandboxConfig:
    """User-facing configuration, converted to a v1 spec on create.

    Fields map 1:1 to the YAML spec. ``secrets`` is a mapping
    ``{env_name_in_guest: host_env_var_name}``: the SDK never receives the
    resolved value — the daemon resolves it via ``valueFrom.envRef``. This
    preserves the invariant that secret values never cross the SDK.
    """

    runtime: str = "python"
    image: str | None = None
    ttl: int = 300
    egress: list[str] = field(default_factory=list)
    memory_mb: int = 512
    cpu_millicores: int = 1000
    disk_mb: int = 1024
    env: dict[str, str] = field(default_factory=dict)
    secrets: dict[str, str] = field(default_factory=dict)
    secret_files: dict[str, str] = field(default_factory=dict)
    working_dir: str | None = None
    prefer_warm: bool = False
    backend: str | None = None
    extensions: dict[str, Any] | None = None

    API_VERSION: ClassVar[str] = "sandbox.ai/v1"
    KIND: ClassVar[str] = "Sandbox"

    def to_spec(self) -> dict[str, Any]:
        """Render the config as the JSON body expected by ``POST /v1/sandboxes``.

        The daemon accepts camelCase fields (see ``SandboxSpec`` in
        ``agentsandbox-core``). Empty/None fields are omitted rather than
        serialised as ``null`` so the daemon's ``deny_unknown_fields`` guards
        on nested structs stay happy.
        """

        runtime: dict[str, Any] = {}
        if self.image:
            runtime["image"] = self.image
        else:
            runtime["preset"] = self.runtime
        if self.env:
            runtime["env"] = self.env
        if self.working_dir:
            runtime["workingDir"] = self.working_dir

        spec_body: dict[str, Any] = {
            "runtime": runtime,
            "resources": {
                "memoryMb": self.memory_mb,
                "cpuMillicores": self.cpu_millicores,
                "diskMb": self.disk_mb,
            },
            "ttlSeconds": self.ttl,
        }

        if self.egress:
            spec_body["network"] = {
                "egress": {
                    "allow": self.egress,
                    "denyByDefault": True,
                }
            }

        if self.secrets:
            spec_body["secrets"] = [
                {"name": name, "valueFrom": {"envRef": host_var}}
                for name, host_var in self.secrets.items()
            ]
        if self.secret_files:
            spec_body.setdefault("secrets", []).extend(
                {"name": name, "valueFrom": {"file": path}}
                for name, path in self.secret_files.items()
            )
        if self.backend or self.prefer_warm:
            scheduling: dict[str, Any] = {}
            if self.backend:
                scheduling["backend"] = self.backend
            if self.prefer_warm:
                scheduling["preferWarm"] = True
            spec_body["scheduling"] = scheduling
        if self.extensions is not None:
            spec_body["extensions"] = self.extensions

        return {
            "apiVersion": self.API_VERSION,
            "kind": self.KIND,
            "metadata": {},
            "spec": spec_body,
        }


__all__ = ["ExecResult", "ExecStreamEvent", "SandboxInfo", "SandboxConfig"]
