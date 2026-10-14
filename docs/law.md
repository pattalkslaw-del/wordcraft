# LawCraft: court profiles

Legal documents are formatted to a court's rules, and those rules are data: one court profile per
court, with the document's sections, its styles and its checks. LawCraft keeps them as JSON files
so a person can hand one round, edit it in a text editor, and let an agent write one; WordCraft
applies them through its own style and layout commands, so what a profile does is what the ribbon,
the command palette, the CLI, the control channel and the MCP server can do — with the same undo
and the same Restrict Editing checks.

Nothing here knows any court's rules except the profile files that ship with it. Bundled:
**missouri-appellate-brief** and **federal-appellate-brief**. Other courts are entered by users or
their agents.

## Turning it on

LawCraft is an opt-in module, off until the user turns it on. Set `law.enabled` in the editing
preferences (`{"editing":{"modules":{"law":true}}}`) — the same flag the desktop app will expose
on a settings page later. Only then are the `law.*` commands installed in the session, which is
how every way into WordCraft reaches them. From a terminal, a headless run honours the same
setting, or takes `WORDCRAFT_LAW=1` for one run:

```
WORDCRAFT_LAW=1 wordcraft-cli run --cmd 'law.profiles.list' --print
WORDCRAFT_LAW=1 wordcraft-cli run --cmd 'law.profiles.apply={"id":"missouri-appellate-brief","doc":"brief"}' --save brief.docx
```

A user's own profiles live in `<settings>/law/profiles/<id>.json` (the same folder the custom
dictionary lives beside), one file per profile.

## Commands

| Command | What it does |
|---|---|
| `law.profiles.list` | Every profile: `[{id, name, jurisdiction, verified, citationPack, docs}]` |
| `law.profiles.get` | `{"id"}`: the whole profile |
| `law.profiles.create` | Create one from the given profile; always saved unverified |
| `law.profiles.update` | `{"id", …}`: replace the given fields, keep the rest; unverified again |
| `law.profiles.validate` | Check a profile (or `{"path"}` for a file): reports every problem, saves nothing |
| `law.profiles.approve` | `{"id"}`: a person's action — the only thing that verifies a profile |
| `law.profiles.import` | `{"path"}`: read a profile file into the store |
| `law.profiles.export` | `{"id", "path"}`: write a profile file |
| `law.profiles.apply` | `{"id", "doc"?}`: apply a document type's styles, page setup and line numbers to the open document |
| `law.checks.run` | `{"id"?, "doc"?, "forExport"?, "format"?, "store"?}`: run the profile's checks for a document type and return `{"profile", "doc", "findings": [{checkId, severity, message, start?, end?}]}`; it never edits the document |

Applying a profile runs WordCraft's own commands (`layout.size`, `layout.margins`,
`layout.lineNumbers`, `insert.pageNumber`, `layout.pageNumberFormat`, `styles.create`,
`styles.modify`) and the whole application is **one undo step**. A style the document already has
is modified in place, the way Modify Style does: the profile's settings are laid over the style's,
so paragraphs that use the style follow it and the caret's own paragraph keeps its style.
Applying an unverified profile works: the flag says whether a person checked the rules, not
whether they may be used.

## The profile file

Schema `1`. Parsing is hostile-input safe: an unknown schema version, an unknown check kind, a
number out of range or a reference to a style, section or document type that isn't there is an
error that says which, and `law.profiles.validate` reports every problem at once rather than one
at a time.

```json
{
  "schema": 1,
  "id": "missouri-appellate-brief",
  "name": "Missouri appellate brief",
  "jurisdiction": "Missouri",
  "verified": true,
  "docs": {
    "brief": {
      "label": "Appellant's initial brief",
      "page": {"size": "Letter", "top": 72, "bottom": 72, "left": 72, "right": 72},
      "lineNumbers": {"enabled": false},
      "pageNumbers": {"position": "bottom", "align": "center", "format": "decimal"},
      "styles": [
        {"name": "Normal", "chr": {"font": "Times New Roman", "size": 13},
         "para": {"lineSpacing": {"rule": "multiple", "value": 1.5}}},
        {"name": "Caption", "basedOn": "Normal", "next": "Body",
         "para": {"lineSpacing": {"rule": "multiple", "value": 1}}},
        {"name": "Block Quote", "basedOn": "Normal",
         "para": {"lineSpacing": {"rule": "multiple", "value": 1}, "indentLeft": 72, "indentRight": 72}},
        {"name": "Body", "basedOn": "Normal", "next": "Body"},
        {"name": "Heading", "basedOn": "Normal", "next": "Body"},
        {"name": "Certificate", "basedOn": "Normal", "next": "Body"}
      ],
      "sections": [
        {"id": "cover", "name": "Cover", "style": "Heading",
         "fields": ["court", "caseNumber", "partyNames", "partyRoles", "attorneyName", "barNumber"]},
        {"id": "jurisdictionalStatement", "name": "Jurisdictional Statement", "style": "Heading"},
        {"id": "certificateOfService", "name": "Certificate of Service", "style": "Certificate"}
      ],
      "checks": [
        {"kind": "wordLimit", "limit": 31000, "source": "Mo. Sup. Ct. R. 84.04(b)(1)", "effective": "2025-01-01"},
        {"kind": "minFontSize", "pt": 13, "source": "Mo. Sup. Ct. R. 84.03", "effective": "2025-01-01"},
        {"kind": "noHyperlinks", "source": "Mo. Sup. Ct. R. 84.03", "effective": "2025-01-01"},
        {"kind": "textSearchablePdf", "source": "Mo. Sup. Ct. R. 84.03", "effective": "2025-01-01"}
      ],
      "source": "Mo. Sup. Ct. R. 84.03, 84.04, 84.06",
      "effective": "2025-01-01"
    }
  }
}
```

The three parts per document type, as in [`LAWCRAFT.md`](../.lawcraft/LAWCRAFT.md):

- **sections** — the skeleton in order. Each has an id, the heading to write, the style its
  paragraphs use and, for the caption, the fields it holds (`court`, `county`, `partyNames`,
  `partyRoles`, `partyAddresses`, `caseNumber`, `attorneyName`, `barNumber`, `attorneyAddress`,
  `telephone`, `email`, `signature`).
- **styles** — page size and margins, line numbers (exactly the arguments `layout.lineNumbers`
  takes, plus `enabled` for "none"), page numbering, and the named styles. A style is a standard
  WordCraft style object: `chr` and `para` are the same `CharProps` / `ParaProps` the `styles.*`
  commands take, with `basedOn` and `next`.
- **checks** — the rules that aren't styles: `wordLimit` (with `excludeSections` for the parts the
  count leaves out), `minFontSize` (a minimum point size and, optionally, a required typeface),
  `minMargins`, `minLineSpacing` (with `exceptStyles` for the single-spaced exceptions),
  `noHyperlinks`, `requiredSections` (the section ids that must be present, in order) and
  `textSearchablePdf`.

Every rule records `source` (the rule it comes from) and `effective` (when it took effect). The
bundled profiles are verified; anything an agent writes is not, until a person approves it.

## Running the checks

`law.checks.run` evaluates a profile's checks against the open document for one document type
(`doc`; the applied profile and document type are used when they are left out) and returns the
findings. It has **no side effects on the document**; by default it also stores the findings so
they show in Review › Proofing like spelling and grammar, and `"store": false` skips that for a
pure query. A finding has a `checkId` (the check's kind or its id), a `severity` (`error`, or
`warning` when something could not be verified), a message and, where the problem is a range in the
document, `start` and `end`.

A profile applied to a document registers its checks to run **before export** (through the
engine's general export-check hook, which carries no legal logic). The export still proceeds:
findings are reported, not blocking.

- The word limit counts the document's own words (body, text boxes, footnotes and endnotes) and
  leaves out the excluded sections; a text box anchored in an excluded section goes with it.
- Line spacing covers the body and the text boxes it shows — text boxes live in their own part
  stories, so a single-spaced box is caught like a single-spaced paragraph. The profile's
  single-spaced exceptions (Caption, Heading, Block Quote, Signature, Certificate) still apply in
  a box, and no exception may go below single spacing.
- `textSearchablePdf` exports the document in memory and inspects the result: a page that draws a
  scan and carries no text (or only a caption beside the scan) is reported as not searchable, and a
  page dominated by a picture whose text cannot be told from its content is a warning, not a
  silent pass. It runs only when the output being written is a PDF.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-13 | minor | Court-profile checks engine: `law.checks.run`, pre-export checks through the engine's export-check hook, text-box-aware line spacing, and a PDF searchability verdict that no longer passes a scan with a caption |

## How it works

`crates/law` (package `wordcraft-law`) sits at L5 next to `wordcraft-zotero`: it depends on
`wordcraft-doc` and `wordcraft-engine` and brings no legal logic into either. The engine holds the
two hooks it needs — `Session::install_commands` (an optional module's commands, never shadowing a
core command's id) and `Prefs::modules` (which optional modules the user turned on) — and
`law.profiles.apply` calls the existing commands rather than formatting anything itself. The
module crate also builds for wasm32-unknown-unknown; the browser build has no settings folder to
keep a user's own profiles in, so it ships without it.
