"""zk2py: a second, independent implementation of the zk2 core specification.

Built from ``spec/`` alone (issue #609): ``spec/core.md``, its JSON schemas,
the fixtures under ``spec/conformance/`` and the scenarios under
``spec/scenarios/``. This package covers the static half of the spec (§1,
§2.2, §3.3, §5.2, §7.3, §9). The live-bus half is a later phase.

Module map, by spec chapter:

- ``lexical``    §1.2  plain chunks, interface ids, instance ids, fingerprints, ULIDs
- ``slug``       §1.4  the injective slug and its decoder
- ``keys``       §1.1  the five key forms: parse and build
- ``templates``  §2.2  templates, shapes, match-then-rank resolution
- ``jcs``              strict JSON reading, RFC 8785 bytes, sha256 ids
- ``shape``            the JSON Schema checker used for E000 and D000
- ``schemas``    §7.3, §9.4  schema artifacts and type references
- ``protoc``     §9.4  FileDescriptorSet bytes from protoc 3.21.12
- ``contract``   §9.1-§9.5  lints, resolution, the canonical form, fingerprints
- ``sets``       §9.2  E035 / E036
- ``bundle``     §9.6  build and verify
- ``history``    §9.7  the history check
- ``descriptor`` §3.3  the descriptor checker (D codes)
- ``cbor``, ``envelope``  §5.2  the error envelope decoder
- ``compat``     §9.7 retention, §9.8  the compatibility classifier
- ``conformance``      the fixture runner
"""
