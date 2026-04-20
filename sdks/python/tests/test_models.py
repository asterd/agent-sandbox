"""Unit tests for SDK value objects.

These do not touch the network; they verify the shape of ``to_spec`` against
the daemon contract (see ``crates/agentsandbox-core/src/spec.rs``).
"""

from __future__ import annotations

from agentsandbox import ExecResult, SandboxConfig


def test_exec_result_success_true_on_zero():
    r = ExecResult(stdout="ok", stderr="", exit_code=0, duration_ms=10)
    assert r.success is True
    assert str(r) == "ok"


def test_exec_result_success_false_on_nonzero():
    r = ExecResult(stdout="", stderr="oops", exit_code=1, duration_ms=5)
    assert r.success is False


def test_to_spec_minimal_uses_preset_and_omits_network():
    spec = SandboxConfig(runtime="python").to_spec()

    assert spec["apiVersion"] == "sandbox.ai/v1alpha1"
    assert spec["kind"] == "Sandbox"
    runtime = spec["spec"]["runtime"]
    assert runtime == {"preset": "python"}
    assert "network" not in spec["spec"]
    assert "secrets" not in spec["spec"]
    assert spec["spec"]["ttlSeconds"] == 300
    assert spec["spec"]["resources"] == {"memoryMb": 512, "cpuMillicores": 1000}


def test_to_spec_image_overrides_preset():
    spec = SandboxConfig(runtime="python", image="python:3.12-slim").to_spec()
    runtime = spec["spec"]["runtime"]
    assert runtime == {"image": "python:3.12-slim"}
    assert "preset" not in runtime


def test_to_spec_egress_builds_network_block():
    spec = SandboxConfig(runtime="python", egress=["pypi.org"]).to_spec()
    assert spec["spec"]["network"] == {
        "egress": {"allow": ["pypi.org"], "denyByDefault": True}
    }


def test_to_spec_env_round_trip():
    spec = SandboxConfig(runtime="python", env={"FOO": "bar"}).to_spec()
    assert spec["spec"]["runtime"]["env"] == {"FOO": "bar"}


def test_to_spec_secrets_use_env_ref_not_raw_values():
    # The SDK must never place raw secret values in the spec; it only names
    # which host env var the daemon should resolve. Regression guard for the
    # "secrets never cross the SDK" invariant (ROADMAP nota #3).
    spec = SandboxConfig(
        runtime="python", secrets={"API_KEY": "HOST_API_KEY"}
    ).to_spec()
    assert spec["spec"]["secrets"] == [
        {"name": "API_KEY", "valueFrom": {"envRef": "HOST_API_KEY"}}
    ]
