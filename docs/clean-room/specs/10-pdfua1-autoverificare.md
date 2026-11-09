# Funcție: Autoverificare a PDF-ului etichetat produs față de condițiile de eșec PDF/UA-1

Sursă publică: PDF Association, „Matterhorn Protocol 1.1" (31 de puncte de control, 136 de condiții de eșec pentru ISO 14289-1 / PDF/UA-1), anunț https://pdfa.org/rules-for-accessible-pdf-matterhorn-protocol-1-1-is-now-available/ . Documentul protocolului în sine NU a fost citit (tentativa Scribd a eșuat, JavaScript). Citite, în schimb, rezumate secundare, 2026-11-09: https://www.powermapper.com/products/sortsite/rules/accmatterhorn/ (listă de identificatori și clasificare mașină/om), https://taggedpdfschool.pdfa.org/matterhorn (descrierea punctelor de control), https://www.pdflib.com/pdf-knowledge-base/pdfua/matterhorn-protocol , https://pdfa.org/climbing-the-matterhorn-an-introduction-to-the-definitive-algorithm-for-pdfua-conformance/ . ISO 14289-1 (plătit) NU a fost citit.
Stare: candidat, cu o rezervă majoră: textul fiecărei condiții de eșec trebuie obținut din documentul oficial înainte de aprobare. Dimensiune: L.

## Scop
Rezultatul etichetat (etichete structurale, titlu, limbă) al programului este autoverificat doar structural. Specificația cere un raport pe baza condițiilor de eșec ale protocolului, ca utilizatorul să vadă ce lipsește pentru PDF/UA-1 și ce trebuie decis de om. Nu se pretinde conformitate: rapoartele sunt indicii.

## Intrări / ieșiri
- Intrare: un PDF produs de program (sau extern).
- Ieșire: pentru fiecare punct de control relevant: condiții testate automat (trecute/eșuate, cu pagina), condiții care cer judecată umană (listate ca „de verificat"), condiții netestate; sumar fără cuvântul „conform" decât dacă nicio condiție mecanică nu eșuează, iar atunci cu mențiunea că cele umane rămân.

## Comportament observabil
Structura protocolului [observat, rezumate]: 31 de puncte de control; 136 de condiții de eșec (89 evaluabile integral de software, 47 cu validare umană, după PDF Association; un instrument terț numără 137 de identificatori listați, 87 mecanice, 48 umane, 2 fără test: diferența de 1 e semnalată de acel instrument); revizia din 2021 a adăugat o condiție [observat]. Se aplică PDF 1.7 și PDF/UA-1, nu PDF 2.0 / PDF/UA-2 [observat].
Puncte de control și ce spun rezumatele (numerotare după lista cu identificatori; rezumatul taggedpdfschool are numerele 26-29 deplasate: [observat discrepanța], se folosește lista cu identificatori):
| Nr. | Titlu | Conținut rezumat |
|---|---|---|
| 01 | Conținut real etichetat | conținut nici marcat real, nici artefact, sau amestecat |
| 02 | Mapare de roluri | etichete nestandard fără mapare la tip standard; cicluri; tipuri standard remapate |
| 06 | Metadate | flux XMP lipsă, fără identificatorul PDF/UA, titlu lipsă sau neclar |
| 07 | Dicționar | DisplayDocTitle lipsă sau fals |
| 08 | Validare OCR | text OCR cu erori semnificative, neetichetat sau marcat suspect (condiții umane) |
| 09 | Etichete adecvate | ordine de citire, imbricare, semantică, sintaxa tabelelor/listelor |
| 10 | Mapări de caractere | coduri de caractere fără Unicode |
| 11 | Limbă naturală declarată | limba nedeterminabilă pentru text, text alternativ, contururi, adnotări, metadate |
| 13 | Grafice | grafice neetichetate Figure, legende, text alternativ lipsă |
| 14 | Titluri | titluri neetichetate, numerotare greșită sau sărită, H7+, mai multe H într-un nod |
| 15 | Tabele | celule de antet fără TH/Scope; non-tabele etichetate tabel |
| 16 | Liste | listă ordonată fără ListNumbering valid |
| 18 | Antete și subsoluri de pagină | nemarcate ca artefacte de paginare |
| 19 | Note și trimiteri | note neetichetate, chei Note ID lipsă/duplicate |
| 20, 21, 25 | Conținut opțional, fișiere încorporate, XFA | configurări fără nume; fișiere fără F/UF; dynamicRender „required" |
| 26 | Securitate | fișier criptat fără cheia P sau cu bitul 10 fals |
| 28 | Adnotări | adnotări fără imbricare Annot, fără descriere alternativă, fără Tabs, widget în afara Form, legături în afara Link |
| 30 | XObject | XObject de referință; Form XObject cu MCID referit de mai multe ori |
| 31 | Fonturi | 30 de verificări: CID, CIDToGIDMap, CMap, program de font neîncorporat sau incomplet, lățimi, codare TrueType, ToUnicode, glife .notdef folosite |
Condiții cu identificatori mecanice, din lista terță [observat]: 01-003/004/005/007; 02-001/003/004; 06-001..003; 07-001/002; 09-004..008; 10-001; 11-001..006; 13-004; 14-002/003/006/007; 15-003; 17-002/003; 19-003/004; 20-001..003; 21-001; 25-001; 26-001/002; 28-002, 004..012, 014..018; 30-001/002; 31-001..009, 011..030. Cu validare umană: 01-001/002/006; 03-*; 04-001; 05-*; 08-*; 09-001..003; 11-007; 12-001; 13-001..003, 005..008; 14-001/004/005; 15-001/002/004/005; 16-*; 18-*; 19-001/002; 22-001; 24-001; 28-001/003/013; 29-001; 31-010.
Textul precis al fiecărei condiții și pașii de testare: [necunoscut].

Aplicabilitate pentru ieșirea programului [dedus]: pagina e imagine plus text invizibil; deci se verifică în special 01 (conținut real/artefact: imaginea paginii ca artefact), 02, 06, 07, 09, 10, 11, 13, 14, 18, 28, 30, 31. Verificările umane (08 OCR, 14-001 titluri, 13 alt-text) apar mereu în raport ca „de verificat" când sunt relevante.

## Cazuri limită și erori
- Fișier fără etichete → raport „neetichetat", fără listă de condiții.
- PDF criptat → punctul 26 înaintea celorlalte.
- Condiție mecanică fără pas de testare cunoscut → apare ca „netestat", niciodată ca „trecut".
- Titlu absent sau egal cu numele fișierului → avertisment sub 06 [dedus din „titlu lipsă sau neclar"].

## Teste de acceptanță
1. PDF cu DisplayDocTitle fals sau lipsă → eșec punct 07 (condițiile 07-001/002).
2. PDF fără flux XMP sau fără titlul documentului în el → eșec punct 06.
3. Etichetă cu rol nestandard fără intrare în maparea de roluri → eșec punct 02.
4. Etichetă Figure fără text alternativ → eșec 13-004 (singura condiție mecanică a punctului 13).
5. Font neîncorporat sau fără ToUnicode folosit în text → eșec punct 31 / 10 (condiția exactă: [necunoscut]).
6. Raportul pentru un PDF care trece toate condițiile mecanice conține totuși secțiunea „de verificat" cu condițiile umane relevante (08, 13, 14-001) și nu conține cuvântul „conform" nemodificat.
7. Un PDF fără etichete → raport „neetichetat".
8. Un PDF 2.0 → raport că Matterhorn 1.1 nu se aplică PDF 2.0 / PDF/UA-2.

## Necunoscute
- Textul și metodele de test ale celor 136 de condiții; sursele sunt secundare, parafrazate de un instrument de preluare.
- ISO 14289-1 necitit; relația exactă cu ISO 32000-1.
- Un validator independent rămâne necesar și nu este numit aici; ce înseamnă „trece" cere decizie umană.
- Termenii de utilizare ai Matterhorn Protocol nu au fost găsiți.

## Checklist de filtru
Spec: 10-pdfua1-autoverificare   Data: 2026-11-09

- [x] Nicio linie de cod, pseudocod sau asamblare copiată
- [x] Niciun nume intern de funcție, variabilă sau clasă din țintă
- [x] Niciun text de UI, asset sau branding din țintă
- [x] Algoritmii sunt descriși prin intrare → ieșire, nu prin structura codului țintei
- [x] Afirmațiile sunt marcate [observat] / [dedus] / [necunoscut]
- [x] Licența/EULA țintei verificată și notată în `1-analiza/jurnal.md` (notată în `docs/clean-room/specs/JURNAL.md`; licență necunoscută)
- [ ] Specificația nu depinde de brevete sau secrete comerciale (neverificat)
- [ ] Aprobată de mine înainte de predare
