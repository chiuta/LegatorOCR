# Funcție: Legarea textului TEI de imaginile paginii (facsimil, suprafețe, zone)

Sursă publică: TEI Consortium, „TEI P5: Guidelines for Electronic Text Encoding and Interchange", capitolul 11 „Representation of Primary Sources", secțiunea despre fascimile digitale (11.1), în ediția arhivată https://vault.tei-c.de/P5/1.3.0/xml/tei/odd/Source/Guidelines/en/PH-PrimarySources.xml (sursa ODD, citită până la caracterul 100000 din 104024) și pagina clasei de atribute att.global.facs https://vault.tei-c.de/P5/4.2.0/doc/tei-p5-doc/en/html/ref-att.global.facs.html (conținut redus). Citite 2026-11-09. Ediția 1.3.0 este veche; diferențele față de ediția curentă sunt [necunoscut].
Stare: candidat. Dimensiune: M.

## Scop
Ieșirea TEI actuală are pagini, paragrafe, tabele, dar nu spune unde se află fiecare pe imaginea scanată. Specificația cere un facsimil care leagă fiecare pagină și bloc de text de zona din imagine, ca un cititor să poată afișa textul lângă imagine sau să evidențieze un paragraf pe scanare.

## Intrări / ieșiri
- Intrare: pentru fiecare pagină: numele fișierului imagine, lățimea și înălțimea în pixeli; lista blocurilor OCR cu dreptunghiul lor în pixeli.
- Ieșire: documentul TEI cu (a) textul, în care elementele poartă un atribut de legătură spre facsimil; (b) un element de facsimil cu câte o suprafață pe pagină, imaginea ei și zone dreptunghiulare pentru blocurile legate.

## Comportament observabil
1. Abordarea simplă: un marcaj de pagină indică prin atributul global de legătură fișierul imagine; imaginea acoperă tot textul până la marcajul următor [observat]. Abordarea recomandată: un element de facsimil care conține imaginile organizate în suprafețe și zone; facsimilul poate sta lângă textul propriu-zis sau în locul lui; poate avea părți „față" și „spate" [observat].
2. Imagini simple se pun în elemente grafice; o suprafață grupează mai multe imagini ale aceleiași zone fizice (ex. rezoluție mare și mică) [observat].
3. Suprafața are un spațiu abstract de coordonate definit de patru atribute: colț stânga-sus (ulx, uly) și colț dreapta-jos (lrx, lry); unitatea nu e legată de măsurători fizice; pixelii sunt alegerea comună [observat]. Programul folosește pixelii imaginii originale, cu originea în colțul stânga-sus [dedus: orientarea axei y nu a fost în fragment; [necunoscut] în text].
4. Zona este un dreptunghi în același spațiu de coordonate ca suprafața-părinte; zonele se pot suprapune, nu trebuie să fie incluse una în alta și pot ieși din suprafață (de ex. legătura cărții) [observat]. Zone neortogonale cer SVG [observat]; programul folosește doar dreptunghiuri.
5. Legarea textului: atributul global de legătură al unui element indică identificatorul unei suprafețe sau zone; exemplele din capitol îl pun pe marcaje de pagină, titluri, paragrafe, cuvinte, figuri [observat]. Programul pune legătura spre suprafață pe marcajul de pagină și spre zonă pe fiecare paragraf, titlu, tabel și figură pentru care are un dreptunghi [dedus].
6. Legătura inversă: atributul de început al suprafeței indică elementul din text unde începe textul ei [observat].
7. Atributul de numerotare al marcajului de pagină păstrează eticheta (de ex. folio) și nu face parte din mecanismul de facsimil [observat].
8. Combinare: pointerii simpli pentru uz de bibliotecă digitală și suprafețe/zone pentru potrivire precisă pot coexista [observat].
9. Identificatorii sunt unici în document; fiecare legătură trebuie să se rezolve la un identificator existent [dedus din natura referinței].

## Formate / protocoale
Dreptunghiul OCR în pixeli (x, y, lățime, înălțime) se convertește în colțuri: (x, y) și (x+lățime, y+înălțime) [dedus]. Documentul rămâne bine format XML; validarea față de schema TEI nu a fost posibilă în această sesiune: [necunoscut] (schema nu a fost obținută; ROADMAP-ul consemnează același lucru).

## Cazuri limită și erori
- Pagină rotită sau decupată de program: coordonatele se raportează la imaginea pe care o referă legătura (cea livrată), nu la originalul [dedus].
- Bloc fără dreptunghi de încredere: fără zonă, doar paragraf fără legătură.
- Pagini fără OCR: suprafață și imagine fără zone.
- Elemente care nu pot conține alte elemente (de ex. paragrafe în ab): regulă de model de conținut deja întâlnită în proiect; cerința se verifică față de schemă când aceasta este disponibilă [necunoscut].
- Căi de imagine: relative la documentul TEI; spații și caractere speciale codificate ca URI [dedus].

## Teste de acceptanță
1. Document cu 2 pagini → exact 2 suprafețe, fiecare cu o imagine și lățimea/înălțimea pixelilor ei în (lrx, lry), cu (ulx, uly) = (0, 0).
2. Fiecare marcaj de pagină are o legătură ce se rezolvă la suprafața corespunzătoare; nicio legătură nu rămâne nerezolvată (verificare mecanică: fiecare valoare de legătură este un identificator existent).
3. Un paragraf cu dreptunghiul OCR (100, 200, 300, 50) → zonă cu ulx 100, uly 200, lrx 400, lry 250 în suprafața paginii lui; paragraful poartă legătura spre zonă.
4. Două paragrafe cu dreptunghiuri suprapuse → două zone suprapuse, ambele păstrate (suprapunerea e permisă).
5. Un bloc cu dreptunghiul parțial în afara paginii → zona păstrată fără decupare (zonele pot ieși din suprafață).
6. Fiecare identificator din documentul final este unic.
7. Suprafața are atributul de început care indică elementul marcajului ei de pagină din text.
8. Fișierul rezultat este XML bine format.

## Necunoscute
- Schema TEI curentă, ediția curentă a capitolului 11, convenția axei y, valorile permise exacte ale atributelor.
- Pagina att.global.facs a dat doar o definiție scurtă; tipul de date și elementele care o primesc nu au fost citite.
- Licența Ghidurilor TEI este [dedus] CC BY / BSD (cunoștință generală, neconfirmată în sesiune).

## Checklist de filtru
Spec: 06-tei-facsimil   Data: 2026-11-09

- [x] Nicio linie de cod, pseudocod sau asamblare copiată
- [x] Niciun nume intern de funcție, variabilă sau clasă din țintă
- [x] Niciun text de UI, asset sau branding din țintă
- [x] Algoritmii sunt descriși prin intrare → ieșire, nu prin structura codului țintei
- [x] Afirmațiile sunt marcate [observat] / [dedus] / [necunoscut]
- [x] Licența/EULA țintei verificată și notată în `1-analiza/jurnal.md` (notată în `docs/clean-room/specs/JURNAL.md`; licența nu a fost observată direct)
- [ ] Specificația nu depinde de brevete sau secrete comerciale (neverificat)
- [ ] Aprobată de mine înainte de predare
