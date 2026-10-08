"""Protobuf artifacts (core.md §9.4): FileDescriptorSet bytes from protoc.

core.md §9.4: each listed file "is compiled alone, with its imports and
without source info, into a FileDescriptorSet holding the file and its
imports in dependency order: ``protoc --include_imports``, without
``--include_source_info``". §9.5 "Portability": the protobuf fixtures assume
protoc 3.21.12. The compiler is found through ``ZK2PY_PROTOC``, else
``protoc`` on ``PATH`` (``bootstrap.py`` pins and provides one).
"""

from __future__ import annotations

import functools
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

from google.protobuf import descriptor_pb2

EXPECTED_VERSION = "libprotoc 3.21.12"

#: core.md §9.4: the well-known types, each its own artifact, compiled on
#: demand. "Other google.protobuf messages are not" available.
WELL_KNOWN = {
    "google.protobuf.Any": "any.proto",
    "google.protobuf.Duration": "duration.proto",
    "google.protobuf.Empty": "empty.proto",
    "google.protobuf.FieldMask": "field_mask.proto",
    "google.protobuf.Struct": "struct.proto",
    "google.protobuf.Value": "struct.proto",
    "google.protobuf.ListValue": "struct.proto",
    "google.protobuf.Timestamp": "timestamp.proto",
    **{
        f"google.protobuf.{w}": "wrappers.proto"
        for w in ("DoubleValue", "FloatValue", "Int64Value", "UInt64Value", "Int32Value",
                  "UInt32Value", "BoolValue", "StringValue", "BytesValue")
    },
}


class ProtocUnavailable(RuntimeError):
    """No protoc could be run: an environment problem, never a lint."""


class CompileError(ValueError):
    """protoc refused the file (E029)."""


@functools.cache
def protoc_path() -> str:
    exe = os.environ.get("ZK2PY_PROTOC") or shutil.which("protoc")
    if not exe:
        raise ProtocUnavailable("no protoc: set ZK2PY_PROTOC or put protoc 3.21.12 on PATH")
    return exe


@functools.cache
def protoc_version() -> str:
    try:
        out = subprocess.run([protoc_path(), "--version"], capture_output=True, text=True, check=True)
    except (OSError, subprocess.CalledProcessError) as e:
        raise ProtocUnavailable(f"cannot run {protoc_path()}: {e}") from e
    return out.stdout.strip()


def compile_file(name: str, roots: list[Path]) -> bytes:
    """``protoc --include_imports -o OUT -I root... name``: the artifact bytes.

    ``name`` is the file's path relative to its import root, which is also
    the artifact's ``name`` (core.md §9.4).
    """
    protoc_version()  # fail early, as an environment problem
    with tempfile.TemporaryDirectory(prefix="zk2py-protoc-") as tmp:
        out = Path(tmp) / "out.pb"
        cmd = [protoc_path(), "--include_imports", f"--descriptor_set_out={out}"]
        cmd += [f"--proto_path={Path(r).resolve()}" for r in roots]
        cmd.append(name)
        # Run from an empty directory so the working directory adds nothing
        # to the import path.
        proc = subprocess.run(cmd, capture_output=True, text=True, cwd=tmp)
        if proc.returncode != 0:
            raise CompileError(proc.stderr.strip() or f"protoc exited {proc.returncode}")
        data = out.read_bytes()
    _refuse_editions(data)
    return data


def compile_well_known(file: str) -> bytes:
    """A well-known type's artifact, ``google/protobuf/<file>``, from the
    compiler's own include directory (SPEC-FINDINGS: well-known sources)."""
    return compile_file(f"google/protobuf/{file}", [])


def _refuse_editions(data: bytes) -> None:
    """core.md §7.1: "Editions are not supported"; §9.4: a file using
    editions does not compile (E029). protoc 3.21.12 predates editions and
    refuses ``edition = …`` itself; this guards a newer compiler."""
    fds = parse_set(data)
    for f in fds.file:
        if f.syntax not in ("", "proto2", "proto3"):
            raise CompileError(f"{f.name}: syntax {f.syntax!r} is not proto2/proto3")


def parse_set(data: bytes) -> descriptor_pb2.FileDescriptorSet:
    fds = descriptor_pb2.FileDescriptorSet()
    fds.ParseFromString(data)
    return fds


def message_names(fdp: descriptor_pb2.FileDescriptorProto) -> set[str]:
    """Fully qualified names of every message a file defines, nested ones
    included (``pkg.Outer.Inner``)."""
    prefix = f"{fdp.package}." if fdp.package else ""
    out: set[str] = set()

    def walk(msgs, scope: str) -> None:
        for m in msgs:
            full = scope + m.name
            out.add(full)
            walk(m.nested_type, full + ".")

    walk(fdp.message_type, prefix)
    return out


def find_message(fds: descriptor_pb2.FileDescriptorSet, full_name: str):
    """(file, DescriptorProto) of a message in a set, or None."""
    for f in fds.file:
        prefix = f"{f.package}." if f.package else ""

        def walk(msgs, scope):
            for m in msgs:
                full = scope + m.name
                if full == full_name:
                    return m
                hit = walk(m.nested_type, full + ".")
                if hit is not None:
                    return hit
            return None

        hit = walk(f.message_type, prefix)
        if hit is not None:
            return f, hit
    return None
