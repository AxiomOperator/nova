# Nova Error Code Registry

Every diagnostic Nova emits has a code listed here. Ranges:

| Range | Area | Spec |
|---|---|---|
| E0001–E0099 | Lexing and parsing | `syntax.md` (P0-02) |
| E0101–E0199 | Names, bindings, arguments | `syntax.md` / Phase 1 types spec |
| E0401–E0499, W0401–W0499 | Effects | `effects.md` §12 |
| E0501–E0599, W0501–W0599 | AI | `ai.md` §16 |
| E0600–E0699 | Entry points and CLI | this file |

Status: ✅ emitted by the current build · ⏳ specified, not yet emitted.

## Syntax (E00xx)

| Code | Status | Meaning |
|---|---|---|
| E0001 | ✅ | Unexpected character |
| E0002 | ✅ | Unknown escape sequence |
| E0003 | ✅ | Unterminated string or interpolation |
| E0004 | ✅ | Triple-quoted strings not supported yet |
| E0005 | ✅ | Unmatched `}` in string (write `}}`) |
| E0006 | ✅ | Expected X, found Y |
| E0007 | ✅ | Integer literal too large |
| E0008 | ✅ | Expected an item / effect member |
| E0009 | ✅ | Construct not supported yet (names the delivering phase) |
| E0010 | ✅ | Missing parameter type |
| E0011 | ✅ | Test name must be a plain string literal |
| E0012 | ✅ | Expected newline or `}` after statement |
| E0013 | ✅ | Invalid assignment target |
| E0014 | ✅ | Empty or malformed string interpolation |
| E0015 | ✅ | `...` placeholder outside documentation examples |

## Names and arguments (E01xx)

| Code | Status | Meaning |
|---|---|---|
| E0101 | ✅ | Cannot find name in scope |
| E0102 | ✅ | Wrong number of arguments / missing arguments |
| E0103 | ✅ | Unknown or duplicate named argument |
| E0104 | ✅ | Assignment to an immutable (`let`) binding |
| E0105 | ✅ | Positional argument after a named argument |
| E0106 | ✅ | Function defined more than once |
| E0107 | ✅ | Expression form not supported yet (field access, methods, function values) |
| E0108 | ✅ | Test defined more than once |

## Effects (E04xx / W04xx)

See `effects.md` §12 for meanings. Status in this build:

| Code | Status | | Code | Status |
|---|---|---|---|---|
| E0401 | ✅ | | E0413 | ✅ (reserved `ffi` name) |
| E0402 | ✅ | | E0414 | ⏳ |
| E0403 | ✅ | | E0415 | ⏳ |
| E0404 | ✅ | | E0416 | ✅ |
| E0405 | ✅ | | E0417 | ✅ |
| E0406 | ✅ | | E0418 | ⏳ |
| E0407 | ✅ (arity only) | | E0419 | ⏳ |
| E0408 | ✅ | | E0420 | ⏳ (needs types) |
| E0409 | ✅ | | E0421 | ⏳ |
| E0410 | ✅ (as E0009) | | E0422 | ⏳ |
| E0411 | ⏳ | | E0423 | ✅ duplicate effect declaration |
| E0412 | ⏳ | | W0401 | ✅ |
| W0402 | ⏳ | | W0403 | ✅ |

## AI (E05xx / W05xx)

All ⏳ — see `ai.md` §16.

## Entry points (E06xx)

| Code | Status | Meaning |
|---|---|---|
| E0600 | ✅ | No `main` function (`nova run`) |
| E0601 | ✅ | `main` must not take parameters |
