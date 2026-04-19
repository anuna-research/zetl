# Obsidian top-50 community plugin classification

Generated: 2026-04-19T02:04:30Z

Classifies the top-50 Obsidian community plugins by install count against
the markers required for zetl's v1 MarkdownPostProcessor shim.

**Data sources:**
- Plugin list: https://raw.githubusercontent.com/obsidianmd/obsidian-releases/master/community-plugins.json
- Install counts: https://raw.githubusercontent.com/obsidianmd/obsidian-releases/master/community-plugin-stats.json (downloads, all versions)
- Per-plugin `main.js` and `manifest.json` from each repo's `releases/latest/download/`

**Grep markers (against minified main.js):**
- MPP: `registerMarkdownPostProcessor`, `registerMarkdownCodeBlockProcessor`
- Editor: `registerEditorExtension`, `registerEditorSuggest`
- View/workspace: `registerView`, `registerHoverLinkSource`, `setViewState` combined with `getLeaf`/`getLeftLeaf`/`getRightLeaf`/`getActiveLeaf`/`createLeafInParent`/`splitLeaf`
- Desktop-only: `manifest.isDesktopOnly === true` (classifier forces these to `other` regardless of markers)

**Classification precedence:** desktop-only → other; else MPP + (editor OR view) → hybrid; else MPP-only → pure-mpp; else editor OR view → editor-or-workspace; else other.

## Per-plugin

| rank | plugin_id | repo | downloads | classification | mpp | editor | view | desktop_only | notes |
|------|-----------|------|-----------|----------------|-----|--------|------|--------------|-------|
| 1 | `obsidian-excalidraw-plugin` | [zsviczian/obsidian-excalidraw-plugin](https://github.com/zsviczian/obsidian-excalidraw-plugin) | 5831602 | **hybrid** | Y | Y | Y | false | MPP + editor/view |
| 2 | `templater-obsidian` | [SilentVoid13/Templater](https://github.com/SilentVoid13/Templater) | 4056653 | **hybrid** | Y | Y | - | false | MPP + editor/view |
| 3 | `dataview` | [blacksmithgu/obsidian-dataview](https://github.com/blacksmithgu/obsidian-dataview) | 3986287 | **hybrid** | Y | Y | - | false | MPP + editor/view |
| 4 | `obsidian-tasks-plugin` | [obsidian-tasks-group/obsidian-tasks](https://github.com/obsidian-tasks-group/obsidian-tasks) | 3319703 | **hybrid** | Y | Y | - | false | MPP + editor/view |
| 5 | `table-editor-obsidian` | [tgrosinger/advanced-tables-obsidian](https://github.com/tgrosinger/advanced-tables-obsidian) | 2717076 | **editor-or-workspace** | - | Y | Y | false | no MPP |
| 6 | `calendar` | [liamcain/obsidian-calendar-plugin](https://github.com/liamcain/obsidian-calendar-plugin) | 2542398 | **editor-or-workspace** | - | - | Y | false | no MPP |
| 7 | `obsidian-git` | [Vinzent03/obsidian-git](https://github.com/Vinzent03/obsidian-git) | 2404797 | **editor-or-workspace** | - | Y | Y | false | no MPP |
| 8 | `obsidian-style-settings` | [obsidian-community/obsidian-style-settings](https://github.com/obsidian-community/obsidian-style-settings) | 2229731 | **editor-or-workspace** | - | - | Y | false | no MPP |
| 9 | `obsidian-kanban` | [obsidian-community/obsidian-kanban](https://github.com/obsidian-community/obsidian-kanban) | 2220678 | **editor-or-workspace** | - | Y | Y | false | no MPP |
| 10 | `obsidian-icon-folder` | [FlorianWoelki/obsidian-iconize](https://github.com/FlorianWoelki/obsidian-iconize) | 1953532 | **hybrid** | Y | Y | - | false | MPP + editor/view |
| 11 | `remotely-save` | [remotely-save/remotely-save](https://github.com/remotely-save/remotely-save) | 1822430 | **other** | - | - | - | false | no MPP, no editor, no view (commands/settings/sync only) |
| 12 | `quickadd` | [chhoumann/quickadd](https://github.com/chhoumann/quickadd) | 1712553 | **editor-or-workspace** | - | - | Y | false | no MPP |
| 13 | `obsidian-minimal-settings` | [kepano/obsidian-minimal-settings](https://github.com/kepano/obsidian-minimal-settings) | 1485736 | **other** | - | - | - | false | no MPP, no editor, no view (commands/settings/sync only) |
| 14 | `omnisearch` | [scambier/obsidian-omnisearch](https://github.com/scambier/obsidian-omnisearch) | 1369109 | **other** | - | - | - | false | no MPP, no editor, no view (commands/settings/sync only) |
| 15 | `editing-toolbar` | [PKM-er/obsidian-editing-toolbar](https://github.com/PKM-er/obsidian-editing-toolbar) | 1340016 | **editor-or-workspace** | - | Y | - | false | no MPP |
| 16 | `copilot` | [logancyang/obsidian-copilot](https://github.com/logancyang/obsidian-copilot) | 1230144 | **editor-or-workspace** | - | Y | Y | false | no MPP |
| 17 | `obsidian-importer` | [obsidianmd/obsidian-importer](https://github.com/obsidianmd/obsidian-importer) | 1171230 | **other** | - | - | - | false | no MPP, no editor, no view (commands/settings/sync only) |
| 18 | `obsidian-outliner` | [vslinko/obsidian-outliner](https://github.com/vslinko/obsidian-outliner) | 1162884 | **editor-or-workspace** | - | Y | - | false | no MPP |
| 19 | `homepage` | [mirnovov/obsidian-homepage](https://github.com/mirnovov/obsidian-homepage) | 1068901 | **editor-or-workspace** | - | - | Y | false | no MPP |
| 20 | `recent-files-obsidian` | [tgrosinger/recent-files-obsidian](https://github.com/tgrosinger/recent-files-obsidian) | 984301 | **editor-or-workspace** | - | - | Y | false | no MPP |
| 21 | `tag-wrangler` | [pjeby/tag-wrangler](https://github.com/pjeby/tag-wrangler) | 930594 | **editor-or-workspace** | - | - | Y | false | no MPP |
| 22 | `smart-connections` | [brianpetro/obsidian-smart-connections](https://github.com/brianpetro/obsidian-smart-connections) | 908460 | **hybrid** | Y | - | Y | false | MPP + editor/view |
| 23 | `obsidian-admonition` | [javalent/admonitions](https://github.com/javalent/admonitions) | 892628 | **hybrid** | Y | Y | - | false | MPP + editor/view |
| 24 | `obsidian-linter` | [platers/obsidian-linter](https://github.com/platers/obsidian-linter) | 867992 | **editor-or-workspace** | - | Y | - | false | no MPP |
| 25 | `obsidian-advanced-slides` | [MSzturc/obsidian-advanced-slides](https://github.com/MSzturc/obsidian-advanced-slides) | 817280 | **other** | Y | Y | Y | true | desktop-only (skipped regardless of markers) |
| 26 | `obsidian-mind-map` | [lynchjames/obsidian-mind-map](https://github.com/lynchjames/obsidian-mind-map) | 802479 | **editor-or-workspace** | - | - | Y | false | no MPP |
| 27 | `make-md` | [Make-md/makemd](https://github.com/Make-md/makemd) | 790474 | **hybrid** | Y | Y | Y | false | MPP + editor/view |
| 28 | `obsidian-day-planner` | [ivan-lednev/obsidian-day-planner](https://github.com/ivan-lednev/obsidian-day-planner) | 770586 | **editor-or-workspace** | - | - | Y | false | no MPP |
| 29 | `obsidian42-brat` | [TfTHacker/obsidian42-brat](https://github.com/TfTHacker/obsidian42-brat) | 664753 | **other** | - | - | - | false | no MPP, no editor, no view (commands/settings/sync only) |
| 30 | `obsidian-livesync` | [vrtmrz/obsidian-livesync](https://github.com/vrtmrz/obsidian-livesync) | 638341 | **editor-or-workspace** | - | - | Y | false | no MPP |
| 31 | `periodic-notes` | [liamcain/obsidian-periodic-notes](https://github.com/liamcain/obsidian-periodic-notes) | 635036 | **other** | - | - | - | false | no MPP, no editor, no view (commands/settings/sync only) |
| 32 | `highlightr-plugin` | [chetachiezikeuzor/Highlightr-Plugin](https://github.com/chetachiezikeuzor/Highlightr-Plugin) | 627518 | **other** | - | - | - | false | no MPP, no editor, no view (commands/settings/sync only) |
| 33 | `obsidian-advanced-uri` | [Vinzent03/obsidian-advanced-uri](https://github.com/Vinzent03/obsidian-advanced-uri) | 568894 | **editor-or-workspace** | - | - | Y | false | no MPP |
| 34 | `better-word-count` | [lukeleppan/better-word-count](https://github.com/lukeleppan/better-word-count) | 567177 | **editor-or-workspace** | - | Y | - | false | no MPP |
| 35 | `obsidian-annotator` | [elias-sundqvist/obsidian-annotator](https://github.com/elias-sundqvist/obsidian-annotator) | 561543 | **hybrid** | Y | Y | Y | false | MPP + editor/view |
| 36 | `obsidian-textgenerator-plugin` | [nhaouari/obsidian-textgenerator-plugin](https://github.com/nhaouari/obsidian-textgenerator-plugin) | 534177 | **hybrid** | Y | Y | Y | false | MPP + editor/view |
| 37 | `advanced-canvas` | [Developer-Mike/obsidian-advanced-canvas](https://github.com/Developer-Mike/obsidian-advanced-canvas) | 528056 | **editor-or-workspace** | - | Y | Y | false | no MPP |
| 38 | `obsidian-markmind` | [MarkMindCkm/obsidian-markmind](https://github.com/MarkMindCkm/obsidian-markmind) | 523760 | **hybrid** | Y | - | Y | false | MPP + editor/view |
| 39 | `pdf-plus` | [RyotaUshio/obsidian-pdf-plus](https://github.com/RyotaUshio/obsidian-pdf-plus) | 521463 | **hybrid** | Y | - | Y | false | MPP + editor/view |
| 40 | `obsidian-pandoc` | [OliverBalfour/obsidian-pandoc](https://github.com/OliverBalfour/obsidian-pandoc) | 497286 | **other** | - | - | - | true | desktop-only (skipped regardless of markers) |
| 41 | `cmdr` | [phibr0/obsidian-commander](https://github.com/phibr0/obsidian-commander) | 492019 | **other** | - | - | - | false | no MPP, no editor, no view (commands/settings/sync only) |
| 42 | `nldates-obsidian` | [argenos/nldates-obsidian](https://github.com/argenos/nldates-obsidian) | 482113 | **editor-or-workspace** | - | Y | - | false | no MPP |
| 43 | `obsidian-hover-editor` | [nothingislost/obsidian-hover-editor](https://github.com/nothingislost/obsidian-hover-editor) | 479870 | **editor-or-workspace** | - | - | Y | false | no MPP |
| 44 | `obsidian-spaced-repetition` | [st3v3nmw/obsidian-spaced-repetition](https://github.com/st3v3nmw/obsidian-spaced-repetition) | 479541 | **editor-or-workspace** | - | - | Y | false | no MPP |
| 45 | `various-complements` | [tadashi-aikawa/obsidian-various-complements-plugin](https://github.com/tadashi-aikawa/obsidian-various-complements-plugin) | 476687 | **editor-or-workspace** | - | Y | - | false | no MPP |
| 46 | `obsidian-zotero-desktop-connector` | [obsidian-community/obsidian-zotero-integration](https://github.com/obsidian-community/obsidian-zotero-integration) | 468639 | **other** | - | - | Y | true | desktop-only (skipped regardless of markers) |
| 47 | `obsidian-emoji-toolbar` | [oliveryh/obsidian-emoji-toolbar](https://github.com/oliveryh/obsidian-emoji-toolbar) | 456498 | **other** | - | - | - | false | no MPP, no editor, no view (commands/settings/sync only) |
| 48 | `notebook-navigator` | [johansan/notebook-navigator](https://github.com/johansan/notebook-navigator) | 449683 | **editor-or-workspace** | - | - | Y | false | no MPP |
| 49 | `url-into-selection` | [denolehov/obsidian-url-into-selection](https://github.com/denolehov/obsidian-url-into-selection) | 445726 | **other** | - | - | - | false | no MPP, no editor, no view (commands/settings/sync only) |
| 50 | `obsidian-latex-suite` | [artisticat1/obsidian-latex-suite](https://github.com/artisticat1/obsidian-latex-suite) | 443894 | **editor-or-workspace** | - | Y | - | false | no MPP |

## Summary

- **total**: 50
- editor-or-workspace: 25 (50.0%)
- hybrid: 12 (24.0%)
- other: 13 (26.0%)

Unclassifiable (fetch failure): 0
