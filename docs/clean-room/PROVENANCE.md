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

## Constatări din audit automat (2026-11-08, doar observații)
- `legatorocr-engine/src/scantailor.rs` citește fișierele de proiect `.ScanTailor` (XML), pentru interoperabilitate. Antetul spune că formatul a fost reconstruit din descrierea publicată și citește doar câteva elemente/atribute. Nu s-a identificat cod ScanTailor în sursă, dar comparația cu codul GPL al ScanTailor nu a fost făcută (nu a existat acces la el în audit).
- ABBYY FineReader și Agent Ransack apar doar în comentarii și documentație, ca referințe de flux de lucru.
- 346 de dependențe. Expresiile cu GPL/LGPL sunt duale cu alternativă permisivă (`self_cell`, `r-efi`).
- **De verificat:** `THIRD_PARTY_LICENSES.md` listează fonturi incluse sub GPL-3+ cu excepție pentru fonturi (GNU FreeFont) și AGPL-3 cu excepție pentru fonturi (URW base35). Confirmă că excepția acoperă distribuirea fonturilor în aplicație și încorporarea în PDF-urile produse.

**Limite ale auditului:** căutarea după nume nu poate detecta cod copiat fără nume. Rămân de răspuns întrebările din secțiunea „Ce a existat până acum".
