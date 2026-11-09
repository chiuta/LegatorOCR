# Funcție: Citire și verificare a zonei de citire automată (MRZ) pentru formatele TD1 și TD3

Sursă publică: ICAO Doc 9303, „Machine Readable Travel Documents", ediția a 8-a, 2021: Partea 3 (reguli comune) https://icao.int/publications/Documents/9303_p3_cons_en.pdf ; Partea 4 (TD3, pașapoarte) https://www.icao.int/publications/documents/9303_p4_cons_en.pdf ; Partea 5 (TD1) https://www.icao.int:443/publications/Documents/9303_p5_cons_en.pdf . Citite 2026-11-09 prin instrument de preluare cu rezumare. Partea 6 (TD2) și Partea 7 (vize) NU au fost citite.
Stare: candidat. Dimensiune: M.

## Scop
Dintr-un text OCR al unei pagini de identitate, programul găsește liniile MRZ, le descompune în câmpuri, verifică cifrele de control și raportează ce nu corespunde, ca indiciu pentru un om (nu dovedește autenticitatea documentului).

## Intrări / ieșiri
- Intrare: text (linii) de la OCR; opțional o indicație de format.
- Ieșire: pentru fiecare bloc MRZ găsit: format (TD1 sau TD3), câmpuri extrase, rezultatul fiecărei verificări (cifră de control declarată, calculată, da/nu), avertismente de conținut (caractere nepermise, date imposibile).

## Comportament observabil
**Alfabet.** Doar A-Z, 0-9 și „<" (umplutura). Fără semne diacritice sau caractere naționale [observat, P3 4.3-4.4]. Câmpurile „litere" conțin doar A-Z, câmpurile „cifre" doar 0-9 [observat, P4 notele elementelor de date].
**Cifra de control** [observat, P3 4.9, Anexa A]: modulo 10, ponderi 7, 3, 1 repetate de la stânga; valori: 0-9 → 0-9, A-Z → 10-35, „<" → 0; cifra este restul sumei produselor la 10.

**Format TD1: 3 linii × 30 caractere** [observat, P5 4.2]:
| Linia | Poziții | Câmp |
|---|---|---|
| 1 | 1-2 | cod document (prima literă A, C sau I) |
| 1 | 3-5 | stat emitent (3 litere) |
| 1 | 6-14 | număr document (9) |
| 1 | 15 | cifră de control pentru 6-14 |
| 1 | 16-30 | date opționale |
| 2 | 1-6 | data nașterii AALLZZ |
| 2 | 7 | cifră de control pentru 1-6 |
| 2 | 8 | sex F, M sau „<" |
| 2 | 9-14 | data expirării AALLZZ |
| 2 | 15 | cifră de control pentru 9-14 |
| 2 | 16-18 | naționalitate |
| 2 | 19-29 | date opționale |
| 2 | 30 | cifră de control compusă |
| 3 | 1-30 | nume: identificator primar, „<<", identificator secundar |
Cifra compusă TD1 acoperă: linia 1 pozițiile 6-30; linia 2 pozițiile 1-7, 9-15 și 19-29. Sunt excluse linia 1 pozițiile 1-5, linia 2 pozițiile 8 și 16-18, și toată linia 3 [observat, P5 4.2.4].

**Format TD3: 2 linii × 44 caractere** [observat, P4 4.2]:
| Linia | Poziții | Câmp |
|---|---|---|
| 1 | 1-2 | cod document (prima literă P) |
| 1 | 3-5 | stat emitent |
| 1 | 6-44 | nume |
| 2 | 1-9 | număr pașaport |
| 2 | 10 | cifră de control pentru 1-9 |
| 2 | 11-13 | naționalitate |
| 2 | 14-19 | data nașterii |
| 2 | 20 | cifră de control pentru 14-19 |
| 2 | 21 | sex |
| 2 | 22-27 | data expirării |
| 2 | 28 | cifră de control pentru 22-27 |
| 2 | 29-42 | număr personal sau date opționale |
| 2 | 43 | cifră de control pentru 29-42 |
| 2 | 44 | cifră compusă |
Cifra compusă TD3 acoperă linia 2 pozițiile 1-10, 14-20 și 22-43; exclude 11-13 și 21 [observat, P4 4.2.4]. Dacă 29-42 sunt nefolosite, cifra de la 43 poate fi „0" sau „<", la alegerea statului [observat, P4].

**Reguli comune.**
- Poziții nefolosite: „<", pornind de la stânga câmpului [observat, P3 4.10].
- Nume: identificatorul primar, apoi „<<", apoi cel secundar; componentele din fiecare separate de un singur „<"; completare cu „<" [observat, P3 4.6]. Reguli de punctuație: apostrof eliminat fără umplutură; cratimă → un „<"; virgulă între identificatori → „<<"; virgulă în altă parte → un „<"; orice altă punctuație eliminată [observat, P3 4.6]. Transliterarea din alte alfabete urmează secțiunea 6 din Partea 3 [observat; conținutul nu a fost citit: [necunoscut]].
- Trunchiere nume: dacă numele depășește câmpul și există identificator secundar, se scurtează primarul până se eliberează 3 poziții pentru „<<" și prima literă a secundarului; ultima poziție a câmpului (TD1 linia 3 poz. 30; TD3 linia 1 poz. 44) trebuie să fie literă, ceea ce semnalează trunchiere; un nume care umple exact și are literă în ultima poziție se consideră trunchiat [observat, P4 4.2.3; P5 4.2.3]. Metoda de scurtare (inițiale, prescurtări, tăiere) rămâne la discreția statului [observat].
- Dată: AALLZZ; părți necunoscute ale datei nașterii se completează cu „<" [observat, P3 4.8]. Secolul nu rezultă din dată: regula de alegere este [necunoscut] (decizie de politică a implementării, ex. data expirării față de data curentă).
- Sex: „F", „M", „<" (nespecificat); în zona vizuală „X" corespunde lui „<" [observat].
- TD1, număr de document mai lung de 9: primele 9 în 6-14, „<" în locul cifrei de la 15, restul la începutul câmpului opțional urmat de cifră de control și „<" [observat, P5 nota j].
- Cod document TD1: prima literă A, C sau I; „V" nu se folosește; „C" nu urmează după „A" (cu excepția certificatului de membru de echipaj) și nu urmează după „I" [observat, P5 anexa B]. TD3: prima literă „P", a doua la alegerea statului sau „<" [observat, P4].
- Cod stat: trei litere conform Părții 3, secțiunea 5; „UTO" pentru specimene [observat]. Lista codurilor nu a fost citită: [necunoscut].

**Căutare în text OCR.** Un bloc este candidat dacă are numărul și lungimea de linii ale unui format și doar caractere din alfabet, după normalizare (litere mici → mari, spații interioare eliminate) [dedus]. Repararea confuziilor de recunoaștere (O cu 0, I cu 1, S cu 5, B cu 8, Z cu 2) se aplică după tipul câmpului, în ambele sensuri: în câmpuri numerice (date, cifre de control) litere confundabile → cifre; în câmpuri literă (stat, naționalitate, nume) cifre confundabile → litere; niciodată în câmpuri alfanumerice (număr de document, date opționale) [dedus din tipurile de câmp; nu este cerință a standardului]. Orice reparație se raportează.

## Cazuri limită și erori
- Cifră declarată ≠ calculată: raport de nepotrivire, nu eșec al găsirii blocului.
- Număr greșit de linii/lungime greșită: blocul nu este recunoscut (nu se ghicesc caractere lipsă) [dedus].
- Caracter nepermis rămas după reparație: avertisment pe câmp.
- Date imposibile (lună 13, zi 32): avertisment [dedus].
- TD2 (2×36), vize MRV-A/B (2×44, 2×36): nespecificate aici [necunoscut]; programul le ignoră.

## Teste de acceptanță
1. Șirul 520727 → cifră de control 3 (exemplul 1 din Anexa A a Părții 3). Șirul AB2134<<< → cifră 5 (exemplul 2).
2. TD1 de specimen: „I<UTOD231458907<<<<<<<<<<<<<<<" / „7408122F1204159UTO<<<<<<<<<<<6" / „ERIKSSON<<ANNA<MARIA<<<<<<<<<<" → format TD1; cifre de control 7 (document), 2 (naștere), 9 (expirare), 6 (compusă), toate corespund; nume primar ERIKSSON, secundar ANNA MARIA (valorile 7, 2, 9, 6 apar în Partea 5, Anexa A; calculul din regula de ponderi le confirmă).
3. Același specimen cu un singur caracter schimbat în numărul documentului → nepotrivire pe cifra documentului și pe cea compusă, nu pe celelalte.
4. TD3: „P<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<" / „L898902C36UTO7408122F1204159ZE184226B<<<<<10" → format TD3; cifre 6, 2, 9, 1 și compusă 0, toate corespund (calculate cu ponderile 7-3-1 și acoperirile de mai sus; liniile sunt construite după structura din Partea 4, nu copiate din ea, deci [dedus]).
5. „Susie Margaret Smith-Jones" → linia 1 TD3 „P<UTOSMITH<JONES<<SUSIE<MARGARET<<<<<<<<<<<<" (44 caractere), iar „Enya Siobhan O'Connor" → „P<UTOOCONNOR<<ENYA<SIOBHAN<<<<<<<<<<<<<<<<<<" (44 caractere) (exemple din Partea 4, 4.2.3.1).
6. Câmp opțional TD3 nefolosit: poziția 43 acceptată ca „0" și ca „<"; orice alt caracter → avertisment.
7. Text OCR cu „UT0" în câmpul statului (câmp definit literă, dar cifra „0" nu e literă în alfabetul câmpului): se raportează caracter nepermis și, dacă politica de reparație e activă, „0" → „O" se aplică și se raportează; în câmpul datei nașterii, un „O" se repară în „0" și reparația este raportată. [dedus: simetria reparației pe tipul câmpului]
8. Bloc cu 29 de caractere pe o linie TD1 → nerecunoscut ca TD1.

## Necunoscute
- Părțile 6 și 7, secțiunea 6 (transliterare) și lista de coduri din Partea 3, secțiunea 5.
- Rezumatele sursei pot omite note; notele „a-k" din tabelele de câmpuri nu au fost parcurse integral.
- Orientarea secolului pentru AA.

## Checklist de filtru
Spec: 03-mrz-icao9303   Data: 2026-11-09

- [x] Nicio linie de cod, pseudocod sau asamblare copiată
- [x] Niciun nume intern de funcție, variabilă sau clasă din țintă
- [x] Niciun text de UI, asset sau branding din țintă
- [x] Algoritmii sunt descriși prin intrare → ieșire, nu prin structura codului țintei
- [x] Afirmațiile sunt marcate [observat] / [dedus] / [necunoscut]
- [x] Licența/EULA țintei verificată și notată în `1-analiza/jurnal.md` (notată în `docs/clean-room/specs/JURNAL.md`; termenii de reutilizare ICAO nu sunt verificați)
- [ ] Specificația nu depinde de brevete sau secrete comerciale (neverificat)
- [ ] Aprobată de mine înainte de predare
