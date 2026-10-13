# LawCraft

Legal tools built into WordCraft. Every change is a normal WordCraft change: it follows AGENTS.md (clean room, no panics, hostile input, every user-visible action a `CommandSpec` with tests, `cargo xtask ci` green) and is written so it can be offered upstream to storytold/wordcraft as its own pull request.

This file and everything under `.lawcraft/` and `.forgejo/` belong to the build loop, not the product. Never edit them in a goal change.

## Architecture

LawCraft is an opt-in first-party module: crate `crates/law`, package `wordcraft-law`, following the `wordcraft-zotero` pattern. It is off until the user turns it on in settings. Core WordCraft supplies the hooks (commands, proofing, MCP tools, palettes, settings); `wordcraft-law` supplies the legal data and rules (court profiles, citation packs, legal dictionaries). Map onto existing features; never duplicate them. Court profile styles are standard WordCraft style objects; line numbers call the existing line-numbering command. If a core hook is missing, add a small general-purpose hook in core with its own tests, with no legal logic in it. Profile and pack files are JSON.

## Court profiles

A court profile is a data file, not code: one file per court. Profiles are loaded, validated, listed, created and updated through commands (so the ribbon, CLI, control channel and MCP server all reach them). An agent can create or update a profile through those commands.

A profile has three parts, organized by document type (for example: original petition, later pleading, motion, brief, appendix):

1. **Sections.** The document skeleton in order. The caption is a section with fields (court, county, party names and roles, party addresses, case number, attorney name, bar number, address, telephone, email, signature block). Picking a court and a document type builds the skeleton.
2. **Styles.** Page size, margins, font and size, line spacing, single-spaced exceptions (caption, headings, block quotations, certificates, signature block), page numbering (position, whether the cover page is numbered, appendix numbering such as A1), named paragraph styles (Caption, Heading, Body, Block Quote, Signature, Certificate), and an optional line-numbers setting (pleading-paper style: on/off, restart each page, count interval, a rule beside the numbers). WordCraft already has line numbering and style sets; a profile turns them on and configures them. Documents use the named styles, so switching a document's court restyles it.
3. **Checks.** Rules that are not styles, run on demand and before export. Examples: word limits per document type, no active hyperlinks, minimum font size, required sections present, text-searchable PDF. A check can depend on document type (example: no party addresses in any pleading after the original petition).

Citation rules are a separate pack that a profile references by id.

**Verification.** A profile created or changed by an agent is saved as unverified. It is usable but marked unverified until a person approves it in the settings page.

**Settings page.** Inside WordCraft. Lists profiles, shows verified/unverified, edits sections, styles and checks, imports and exports profile files, and approves unverified profiles.

**Included profiles: Missouri and federal only.** Other courts are entered by users or their agents.

- Missouri appellate briefs (Rules 84.03, 84.04, 84.06): sections in order: cover, table of contents, table of authorities, jurisdictional statement, statement of facts, points relied on, argument, conclusion, certificate of compliance, certificate of service; appendix is a separate document numbered from A1. Styles: Times New Roman 13 pt minimum, margins not less than 1 inch, line spacing not less than 1.5 with the single-spaced exceptions above, page numbers at the bottom of every page including the cover. Checks: word limits (appellant's initial brief 31,000; respondent's brief 27,900; reply brief 7,750), no active hyperlinks in Supreme Court filings, text-searchable PDF.
- Federal: build from the Federal Rules of Appellate Procedure and Civil Procedure formatting rules; record each rule's source in the profile.

Every rule in an included profile records its source (rule number) and effective date.

## Legal proofing

- **Legal spell check.** A legal word list added to WordCraft's offline spelling, plus confusion checks for legal word pairs (statute/statue, principal/principle, counsel/council, affect/effect and similar), reported as proofing suggestions.
- **Legal symbols.** A palette and keyboard shortcuts for §, §§, ¶, ¶¶ and related marks, with correct non-breaking spacing after § and ¶.

## Citation check

Checks citation form and reports suggestions; it does not verify that an authority exists.

- Missouri sources follow the forms in Show Me Citations (for example `Thummel v. King, 570 S.W.2d 679 (Mo. banc 1978)`, `(Mo. App. E.D. 2020)`, RSMo and Supreme Court Rule citations).
- Federal sources follow Bluebook forms.
- Encode the citation formats as rules. Never copy text from either manual into the code or data.

## Reference material (not in this repository)

Show Me Citations, 7th ed.; Missouri Court of Appeals Eastern District "ABC's of Appellate Practice" (July 2026); Western District "Quick Guide to Appellate Practice" (rev. July 2026); 2nd Judicial Circuit Local Rules (Rule 3.1 caption, Rule 3.2 paper size 8.5 x 11).
