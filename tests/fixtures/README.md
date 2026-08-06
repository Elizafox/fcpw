# CBOR test fixtures

`appendix_a.json` is copied from the CBOR Working Group's
[`cbor/test-vectors`](https://github.com/cbor/test-vectors) repository. It is
the machine-readable form of the RFC 7049 Appendix A examples, which remain
wire-compatible with RFC 8949. The exact source revision is
`aba89b653e484bc8573c22f3ff35641d79dfd8c1` (2014-01-21).

The fixture is vendored intentionally: tests do not access the network and the
upstream corpus has been stable since 2013.
