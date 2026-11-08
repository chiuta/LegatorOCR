# Proveniență și clean-room — LegatorOCR (chiuta/LegatorOCR)

Document de completat cu fapte reale. Nu declara clean-room pentru ce nu s-a întâmplat așa.

## Ce a existat până acum (completează onest)
- Ținta/țintele: ScanTailor (post-procesare scanări) și, pentru funcții recomandate, ABBYY FineReader / Agent Ransack
- Cum s-a documentat comportamentul țintei (documentație publică, utilizare, formate observate, decompilare?): ______
- A fost folosit vreun cod, pseudocod, decompilare sau asset al țintei în sesiunile de dezvoltare? DA / NU / NU ȘTIU: ______
- Sesiunile de dezvoltare au avut vreodată acces la binarul sau codul țintei? ______

## Risc de licență
ScanTailor este GPL: NU copia cod și nu porta fragmente, altfel proiectul MIT devine derivat GPL. ABBYY FineReader și Agent Ransack sunt proprietare: verifică EULA.

## Dependențe terțe
Rulează `cargo license` (sau `cargo deny check licenses`) și atașează rezultatul. Verifică compatibilitatea cu licența proiectului.

## Pentru lucrul de acum înainte
Folosește fluxul din acest kit (1-analiza → 2-filtru → 3-implementare). Orice funcție nouă inspirată de o țintă trece prin specificație și filtru.

## Recomandare
Pentru lansări comerciale sau donații semnificative, cere o opinie de la un avocat de proprietate intelectuală și fă căutare de mărci (EUIPO), cum ai făcut pentru OSATURA.

## Constatări din audit adversarial (2026-11-08) — fapte verificate în repo

**Corecturi la versiunea anterioară a acestei secțiuni:** (1) am spus că fonturile GPL-3+/AGPL-3 „sunt incluse"; de fapt nu există fișiere de font urmărite în repo, iar PDF-urile folosesc un font generat de cod propriu (vezi mai jos). (2) Am spus că nu s-a comparat cu ScanTailor; comparația structurală parțială e descrisă mai jos.

### Istoric
- 448 de commit-uri pe `main` (2026-10-31 → 2026-11-08), istoric complet. Autori: Alexio/Alexandru-Ionuț Chiuță/alexio (trei nume, aceeași identitate), 18 commit-uri `Claude`, dependabot, merge-uri web. Aproape toate poartă `Co-Authored-By: Claude`: codul e în esență scris cu un model AI, a cărui expunere la cod GPL în antrenare nu poate fi exclusă.
- Nicio adăugare masivă suspectă; primul commit are ~2.500 de linii. „ScanTailor project import" apare prima dată la 2026-11-03; înainte, nimic din `crates/` nu menționează ScanTailor.
- Șterse din istoric: crate-urile `croitor-*` (redenumire), arhive/binare `dist/` vechi (rămân în istoric), fișiere sintetice. Nimic care să indice material terț.

### ScanTailor (risc principal: GPL)
- **Nu s-a găsit contaminare demonstrabilă:** niciun antet, comentariu „adapted/ported", URL către cod terț sau constantă distinctivă ScanTailor în `crates/`, `tools/`, `scripts/`, `web/`.
- S-a comparat structural (nu linie cu linie) cu fișiere publice din `scantailor/scantailor` (deskew, binarizare, content box, layout, dewarp). Rezultat: deskew-ul folosește același criteriu clasic (suma pătratelor diferențelor dintre sumele pe rânduri) și același maxim implicit de 7,0°, ambele răspândite în literatură; implementarea diferă (grilă 0,5°/0,05°/0,01°, proiecție prin tangentă, verificare pe axa verticală). Sauvola: k=0,3 aici față de 0,34; structură diferită. Content box și dewarp: algoritmi diferiți. Constantele distinctive ScanTailor nu apar.
- Numele etapelor (orient, split, deskew, content, dewarp, binarize) urmează fluxul public cunoscut, nu structura internă ScanTailor.
- `scantailor.rs` citește doar câteva elemente din proiectele `.ScanTailor` (directoare, fișiere, pagini, rotație), parser propriu, și spune explicit ce NU preia. Numele de elemente coincid cu schema fișierului, necesar pentru interoperabilitate. Notează în jurnal sursa exactă a descrierii de format.
- **Rămâne neverificabil:** memorarea de structuri GPL de către modelul AI care a scris codul; nu s-a rulat un test automat de similaritate (MOSS/scancode).
- Recomandare: adaugă citări (Otsu 1979, Sauvola & Pietikäinen 2000, Wolf & Jolion 2004) în antetele `binarize.rs` și `deskew.rs`.

### „Clean-room"
- `README.md:270-277` spune „no source code of any existing scan-processing tool was consulted or copied". Nu există dovezi procedurale: acest `PROVENANCE.md` era necompletat, iar `evidence/`, `spec/`, `src/` sunt goale. Termenul apare și în GUI (`i18n.rs`), `CONTRIBUTING.md`, 8 limbi.
- Formulare apărabilă până la completare: *implementare independentă; în sesiunile de dezvoltare nu s-a consultat cod terț (neverificat independent)*. Nu folosi „clean-room" în sens strict.

### Date de antrenare și modele (cel mai sensibil)
- `multi.lnm` e compilat în fiecare binar (`legatorocr-nocr/src/lib.rs:53`). Conform `docs/audit/assets-licences.md`, a fost reantrenat pe linii din numere ale *Revistei StrING* (etichete din stratul text ABBYY sau acord Tesseract+model) și din 98 de pagini ale unui manual din 1960 („Tehnologia producerii mobilei"). Scanările nu sunt în repo, deci nu s-a putut verifica. Drepturile asupra manualului și ale autorilor individuali din revistă trebuie confirmate de titular; regimul antrenării pe opere protejate e juridic nesettled.
- Două modele „candidat" sunt comise (`models/candidate-books.lnm`, `candidate-digital.lnm`), antrenate pe PDF-uri ale unor terți, și **nu apar în nicio listă de licențe** (`THIRD_PARTY_LICENSES.md`, `docs/audit/assets-licences.md`). Nu sunt compilate în binar. Elimină-le din repo public sau documentează sursa și drepturile.
- Cinci fișiere `docs/experiments/mobila1960_*_hand_reference.txt` (966 de cuvinte) conțin transcrieri verbatim din manualul din 1960. Scoate-le din repo public sau păstrează doar hash-uri/liste de cuvinte.
- Opt liste `lexicons/*.txt.gz` (wordfreq, CC BY-SA 4.0) sunt compilate în binarul MIT; atribuirea există. Efectul share-alike asupra ponderilor modelului e nesettled; se evită cu `--no-default-features`.
- Etichetele din stratul ABBYY: EULA ABBYY nu a fost citit (risc probabil mic).

### Fonturi
- Zero fișiere de font urmărite. GNU FreeFont (GPL-3+ cu excepție) și URW base35 (AGPL-3 cu excepție) sunt folosite doar ca surse locale pentru randarea liniilor sintetice de antrenare (`tools/ocr-train/synth.py`). PDF-urile folosesc `GlyphLessFont` generat de cod propriu; `/Helvetica` doar numit. Singurele fonturi livrate sunt cele din egui (GUI). Întrebarea excepției pentru fonturi se pune doar pentru antrenare.
- Numele „GlyphLessFont" coincide cu cel din randorul PDF Tesseract (Apache-2.0); octeții nu au fost comparați.

### Dependențe și packaging
- 354 de pachete în `cargo metadata`; predominant MIT/Apache-2.0. Expresiile cu GPL/LGPL (`self_cell`, `r-efi`) sunt duale cu alternativă permisivă. `djvu-rs` (MIT) declară că e scris din specificație.
- `Dockerfile` instalează `poppler-utils` (GPL) și `tesseract-ocr` ca procese externe; dacă publici imaginea, adaugă notificări și oferta de sursă. CI descarcă veraPDF fără checksum; PyMuPDF (AGPL) e folosit doar în teste.
- `COMPARISON.md` secțiunea 2 are cifre CER depășite față de `ROADMAP.md`; sincronizează sau datează.
- Co-autorul Alexandru Dumbravă apare în `Cargo.toml` și README (cu ORCID); `LICENSE` numește doar „Alexio". Confirmă acordul pentru calitatea de co-autor și pentru publicarea ORCID.
- Mărci (ScanTailor, ABBYY, Agent Ransack, DocFetcher, PDF24): doar referințe nominative; fără sigle sau texte de marketing. Verifică numele „LegatorOCR" într-o căutare de mărci.

### Completat din istoricul conversațiilor Claude (2026-11-08)
Sursa: conversația claude.ai 0f5714f8-… („Clon ScanTaylor în Rust", 856 ture; citită parțial).
- **Acces la ScanTailor:** în ture citite, prima versiune a README a declarat că nu a fost consultat cod ScanTailor; sesiunea a pornit din cererea titularului. Importul `.ScanTailor` e doar un parser XML al fișierelor utilizatorului. Sesiunea de audit din acest PR (nu cea de dezvoltare) a citit fișiere publice ScanTailor doar pentru comparație, deci o eventuală sesiune viitoare de implementare trebuie să fie nouă și separată. Ture necitite: neconfirmat. Titularul confirmă: ______
- **Date de antrenare (documentat în chat):** set de 5956 de linii: 2445 din revista nr. 06 (etichete ABBYY), 3085 din 98 de pagini ale manualului „Tehnologia producerii mobilei" (1960, zip încărcat de titular), 267 din revistele 04/05/07, etichete Tesseract/acord între motoare. Pagini de test din aceeași carte. Un fine-tuning ulterior a fost respins (ROADMAP 4.8), dar `multi.lnm` din build provine din antrenări anterioare. Drepturile asupra revistei și manualului: nedocumentate, de stabilit de titular: ______
- **Co-autor/credit:** portofoliul și blogul lui Alexandru Dumbravă (dumbravaalex.github.io/Portofoliu) sunt creditate în README/site/About, la cererea titularului; au inspirat funcții (căutare, profiluri de flux, sume SHA-256), nu cod. Contribuția și acordul lui: ______
- **Imaginea `02_spread_picture.jpg`:** origine nedeterminată în istoric: ______

**Limite:** comparația cu ScanTailor a fost parțială (doar ramura `master` și doar fișiere selectate); nu s-a rulat test automat de similaritate; nu s-au citit toate cele 163 de fișiere `.rs`; videoclipurile și modelele nu au fost inspectate. Nu este opinie juridică.
