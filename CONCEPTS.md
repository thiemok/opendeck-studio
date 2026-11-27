# Concepts

> Shared domain vocabulary for this project — entities, named processes, and status concepts with project-specific meaning. Seeded with core domain vocabulary, then accretes as ce-compound and ce-compound-refresh process learnings; direct edits are fine. Glossary only, not a spec or catch-all.

## Case models

### Deck variant
One candidate physical shape of the deck, defined by its key grid (for example the 4×8 block or the 1×16 strip), from which every case part of that shape is derived.

### Tile
One separately printed piece of a deck variant's face plate, cut so it fits the printer bed and joined to its neighbours with dovetails.

Cuts fall only in the gaps between key columns, so no key mark is ever split across two tiles.

### Side tile
The tile that carries the OLED and dial module and any extra dials, separate from the key area; which end it joins sets which side the dials are on. A layout that is the same when turned round needs one print for both ends; otherwise each end has its own print.
*Avoid:* side section (when meaning the printed piece)

### Print set
A named collection of deck variants and their parts that one build produces, checks and hands over for printing together, each part with the quantity to print.

## Flagged ambiguities

- "P4" had been used both for the PoC's fourth stage (OTA and wrap-up) and for the ESP32-P4 chip. Plain "P4" means the stage; the board is written "ESP32-P4" or "P4-ETH".
