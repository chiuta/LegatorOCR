# Refilmarea clipurilor GUI LegatorOCR (c01–c11)

Clipurile actuale au fost filmate la 0.2.0-pre.30; versiunea curentă este 0.2.0-pre.72. Fiecare clip are mai jos lista de cadre: ora din clipul actual (când apare subtitrarea) și textul englezesc. Rulezi programul, faci acțiunea descrisă și compari cu clipul vechi; dacă interfața e diferită, refilmezi clipul.

## Cum filmezi (Linux, X11)

```sh
scripts/record-gui-clip.sh c03   # 1360x860, 25 fps, fereastra LegatorOCR cu colțul în 0,0; oprești cu q
```

Subtitrările `cNN.<limbă>.vtt` rămân valabile dacă ordinea și durata pașilor se păstrează; altfel se reajustează orele (textele se traduc deja în 8 limbi).

Apoi: `ffmpeg -i cNN.raw.mkv -c:v libx264 -preset veryfast -crf 26 -pix_fmt yuv420p -movflags +faststart cNN.mp4` și o captură pentru `cNN.jpg`. În docs/index.html textul „versiunea 0.2.0-pre.30” se schimbă în pre.72 (rândurile 114 și 245) abia după refilmare.

## c01 — 81 s

- 0:00 — Start: the window opens on a welcome card with the three steps.
- 0:04 — Press “Add scans…” (or the Add button) to pick images, a PDF or an archive.
- 0:08 — The system file dialog opens; here we type the path and confirm.
- 0:20 — The page is processed at once: a crooked double sheet is split, straightened and cropped.
- 0:25 — More files: File ▸ Add images… accepts several at a time.
- 0:41 — File ▸ Add folder… takes every image of a folder.
- 1:00 — The page list: select a page to preview it; Up and Down change the order.
- 1:09 — Remove deletes the selected page from the job; Clear empties the list.
- 1:16 — The list is empty again and the welcome card is back.

## c02 — 99 s

- 0:00 — Seven different scans are loaded. Click a page in the list to preview it.
- 0:06 — Side by side shows the original and the result; here a double sheet becomes two pages.
- 0:09 — Original shows only the scan, Processed only the result.
- 0:17 — Slider overlays the two: drag the divider to compare before and after.
- 0:26 — Fit scales the preview to the window; switch it off to see real pixels.
- 0:35 — Pick other kinds of pages: a single page with punch holes, a blank page, a negative…
- 0:52 — …a sideways page (it is turned automatically, but may end upside down: the Rotate buttons fix that, see the next clip), and a phone photo.
- 0:57 — The keyboard works too: arrows and Page Up/Down move between pages, Home and End jump to the first and last.
- 1:08 — Ctrl+K opens the command palette: type a few letters and press Enter.
- 1:19 — Pages that need a look are flagged: the bar shows how many, “Next” jumps to the next one (key N).
- 1:25 — The command palette can also show only the flagged pages in the list.

## c03 — 75 s

- 0:00 — The bar above the preview adjusts only the selected page. First, the sideways scan.
- 0:05 — It came out upside down: one Rotate left 90° sets it right.
- 0:11 — Rotate right 90° turns the other way.
- 0:22 — Now the crooked double sheet: -45° and +45° set a diagonal skew of 45° (for pages photographed at an angle)…
- 0:34 — …and Manual skew lets you set the exact angle yourself instead of the automatic one.
- 0:38 — Split this sheet: decide for this sheet alone how it is cut.
- 0:42 — Automatic, at the middle, or not at all.
- 0:45 — Output of this page: give one page another colour mode than the rest.
- 0:49 — Content box (mm): move each side of the crop outwards (+) or inwards (−).
- 1:03 — Undo and Redo step through your changes on this page; Reset brings the page back to the automatic result.

## c04 — 79 s

- 0:00 — Draw zones: mark areas on the original image by dragging. Tick “Draw zones”, then pick a kind.
- 0:06 — Erase: everything inside the rectangle is painted with the paper colour (a stain, a stamp).
- 0:13 — Picture: a rectangle that stays in colour and detail while the rest may become black and white.
- 0:21 — No picture: a rectangle where automatically found pictures are taken out again.
- 0:28 — Right-click a rectangle to remove it; Undo and Redo step through the zones; Clear zones removes them all.
- 0:39 — Guide line: drag along a bent text line; “Straighten curved lines” then flattens the page along it.
- 0:46 — Work area: everything outside the rectangle is painted with paper colour (shadows, a neighbour page).
- 0:53 — Cut line: click or drag on the original: the double sheet is cut there instead of in the automatic place.
- 1:02 — Content box: tick it to edit the crop. Drag the green edges on the processed page.

## c05 — 101 s

- 0:00 — The Settings panel on the right applies to all pages. Input comes first.
- 0:03 — Resolution (dpi): the resolution of the scans; it decides how sizes in millimetres are read.
- 0:16 — Detect sideways pages turns a page that was scanned on its side; Orientation via OCR uses text recognition to tell up from down.
- 0:26 — Page split: Automatic finds the gutter, At the middle cuts in half, Off keeps the sheet whole.
- 0:31 — Geometry: Auto-deskew straightens the page; Max skew limits the angle it may correct.
- 0:42 — Margins: the paper left around the content, in millimetres.
- 0:47 — Page size: Free, a standard size, or a custom one (content is centred on it).
- 0:52 — Output: the colour mode of the result. Gray…
- 0:57 — …Black & white (with threshold options)…
- 1:03 — …Mixed keeps pictures in colour and text in black and white; Hybrid keeps a colour background layer.
- 1:16 — Even out lighting flattens uneven light and shadow; Straighten curved lines flattens a bent page.
- 1:31 — Undo and Redo at the top of the panel step through the settings changes.

## c06 — 90 s

- 0:00 — Clean-up options. To make room, the other sections are folded: click a section title to fold or unfold it.
- 0:07 — Invert negative scans: white text on black (microfilm) becomes normal.
- 0:15 — Auto contrast (levels) stretches dull scans to the full range.
- 0:23 — Remove show-through erases print seen from the other side of thin paper; Smooth halftone (descreen) removes the screen dots of printed pictures.
- 0:35 — The two page 2 options: first the punch holes. Select the page with holes and tick Remove punch holes.
- 0:46 — Phone photos: pick the photo, then Fix perspective straightens the page, and Wipe the background removes the table around it.
- 1:02 — Remove fingers at the edge paints out a thumb holding the page, without touching the print.
- 1:09 — Drop colour removes one ink colour before black-and-white conversion; Sharpen makes the text crisper.
- 1:18 — Erase zones takes rectangles in millimetres (x,y,w,h) as text, the same ones you can draw with the mouse.

## c07 — 82 s

- 0:00 — PDF options. The first three sections are folded to make room.
- 0:05 — JPEG quality: how strongly colour and gray pages are compressed (0 = lossless). Output dpi lowers the resolution to shrink the file.
- 0:14 — Skip blank pages leaves empty pages out of the PDF; Same size for all pages gives every page one size.
- 0:21 — Skip consecutive duplicate pages drops a page identical to the one before it.
- 0:26 — Split PDF at blank pages and at bar-code separator sheets (Code 128) make several PDFs from one batch.
- 0:33 — Bookmarks adds one bookmark per source file; PDF/A-2b writes an archival PDF with embedded font and colour profile.
- 0:40 — Front-matter pages numbers the first pages with roman numerals; Target PDF size aims for a file size in megabytes.
- 0:50 — PDF author and a password for opening protected input PDFs.
- 0:55 — OCR: tick “Searchable text (OCR) in PDF” to add an invisible text layer; choose the recognition languages.
- 1:00 — Also save text (.txt) and Also save hOCR write the recognised text next to the PDF.
- 1:15 — Segmentation mode tells the recognizer how the page is laid out (columns, single block, single line…).

## c08 — 44 s

- 0:00 — Profiles: ready-made sets of settings. Open the Profile list at the top of the panel.
- 0:03 — The built-in profiles cover common jobs: book scans, phone photos, archive copies, and more.
- 0:11 — Choosing one applies all its settings at once.
- 0:19 — Undo and Redo at the top of the panel step back and forth through such changes.
- 0:28 — Your own profile: set things the way you like, type a name and press Save profile.
- 0:36 — The saved profile now appears in the list and can be applied any time.

## c09 — 101 s

- 0:00 — When the pages look right, export. File ▸ Export PDF… (or the button at the bottom of the settings).
- 0:06 — Type a name and confirm; the pages are processed and the PDF is written.
- 0:23 — The status bar reports the result: pages written.
- 0:26 — File ▸ Export images… saves the processed pages as image files instead.
- 0:33 — Pick the folder to write them into.
- 0:51 — File ▸ Save project… keeps the pages and every setting in one file, to continue later.
- 1:02 — File ▸ Save project to folder… also copies the page images into the folder.
- 1:20 — File ▸ Open project… loads a saved project: pages, settings and the per-page adjustments.

## c10 — 45 s

- 0:00 — All options opens a window with every command-line option, in plain controls, grouped by topic.
- 0:04 — Click a topic to unfold it: here the options for page order.
- 0:12 — The search box finds an option by name or by a word of its description.
- 0:17 — Tick an option, and the equivalent command line appears at the bottom.
- 0:21 — Only chosen shows just what you ticked; Copy command puts the command line on the clipboard.
- 0:28 — Run starts the command; its output appears here. --dry-run only lists what would be done.
- 0:36 — Clear choices unticks everything; the cross closes the window.

## c11 — 56 s

- 0:00 — Language: the interface speaks eight languages. Open the Language menu.
- 0:07 — The whole window changes at once; the choice is remembered.
- 0:17 — Help ▸ About shows the version and credits.
- 0:28 — File ▸ Optional modules… checks which extra tools are installed; none is required, the OCR engine is built in.
- 0:39 — Undo and Redo also work from the keyboard: Ctrl+Z and Ctrl+Y.
- 0:50 — Files can also be dropped into the window from a file manager, and the program can be started with files on its command line.
