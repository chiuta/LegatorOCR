# Funcție: Încadrarea unor măsurători de digitizare în niveluri de calitate (de la 1 la 4 stele), pentru documente nelegate

Sursă publică: Federal Agencies Digital Guidelines Initiative (FADGI), „Technical Guidelines for Digitizing Cultural Heritage Materials: Creation of Raster Image Files", septembrie 2016 (ed. Thomas Rieger), https://www.digitizationguidelines.gov/guidelines/ (fișierul „FADGI Federal Agencies Digital Guidelines Initiative-2016 Final_rev1.pdf"). Citit 2026-11-09 prin instrument de preluare cu rezumare: primele 100000 din 131480 de caractere. Ediția a treia (2023, cu revizie 2024) este cea curentă și NU a fost citită; valorile de mai jos pot diferi.
Stare: candidat. Dimensiune: M.

## Scop
Date fiind valorile măsurate pe o scanare (rezoluție, răspuns tonal, balans de alb, uniformitate, eroare de culoare, claritate, zgomot), programul spune ce nivel (1-4 stele) atinge fiecare și care parametru limitează. SPECIFICAȚIA NU DESCRIE METODA DE MĂSURARE (vezi SKIPPED.md): ia măsurătorile ca intrare.

## Intrări / ieșiri
- Intrare: categoria materialului (aici doar „documente nelegate, colecții generale") și valorile măsurate cu unitățile lor.
- Ieșire: pentru fiecare parametru, nivelul atins (0 dacă sub 1 stea) sau „nemăsurat"; lista parametrilor limitativi. Regula de combinare într-un nivel global: [necunoscut] (nu apare în fragmentul citit).

## Comportament observabil
Tabel „Documents (Unbound): General Collections" (p. 26 din ediția 2016) [observat]:
| Parametru | 1 stea | 2 stele | 3 stele | 4 stele |
|---|---|---|---|---|
| Rezoluție (ppi) | 150 | 300 | 300 | 400 |
| Răspuns tonal (OECF), luminanță, nivele pe 8 biți | +9 ≤ 8 | +9 ≤ 8 | +6 ≤ 5 | +3 ≤ 2 |
| Eroare balans de alb, luminanță (nivele) | +8 ≤ 8 | +6 ≤ 6 | +4 ≤ 4 | +3 ≤ 2 |
| Neuniformitate de iluminare (%) | < 8 | < 5 | < 3 | < 1 |
| Acuratețea culorii, ΔE2000 mediu | < 10 | < 8 | < 5 | < 4 |
| MTF10: eficiență de eșantionare (%) | > 60 | > 70 | > 80 | > 90 |
| MTF10: SFR la jumătate din frecvența de eșantionare | < 0,4 | < 0,4 | < 0,4 | < 0,4 |
| MTF50: % din jumătatea frecvenței de eșantionare | [30, 85] | [30, 85] | [35, 75] | [40, 65] |
| Acuratețea scării de reproducere (% din țintă) | ±5 | ±3 | ±2 | ±1 |
| Ascuțire (MTF maxim) | < 1,3 | < 1,2 | < 1,1 | ≤ 1,0 |
| Zgomot, abatere standard ΔL* (nivele) | < 6 (< 4) | < 5 (< 3) | < 4 (< 2) | < 3 (< 1) |
Note: valorile sunt echivalente pe 8 biți [observat]. În sursă rândul de zgomot al acestui tabel are „>" în loc de „<"; celelalte tabele au „<", deci „>" pare greșeală de tipar [observat/dedus]. Se dă doar o limită medie pentru ΔE2000, fără maxim [observat]. Semnificația notației „+9 ≤ 8" (probabil: toleranță sau abatere față de valoarea țintă, plus un maxim) și a perechii din paranteză la zgomot: [necunoscut].
Reguli:
1. Un parametru atinge nivelul cel mai înalt ale cărui condiții le satisface; pentru nivelurile ordonate monoton, nivelul e cel mai mare al cărui prag e respectat [dedus].
2. Intervalul „[a, b]" de la MTF50 e închis la ambele capete [dedus din notația cu paranteze pătrate].
3. Alte categorii de materiale (legate, fotografii, transparențe) au valori diferite (paginile 18-21, 33, 36-39) și nu sunt specificate: [necunoscut].
4. Măsurătorile se bazează pe ținte și programe de analiză publicate de FADGI; definiția SFR/MTF este dată prin trimitere la standardele ISO 12233, ISO 16067-1 și ISO 16067-2, necitite: [necunoscut].

## Cazuri limită și erori
- Valoare lipsă → „nemăsurat", nu zero stele.
- Valoare la limită (de ex. neuniformitate exact 3 %) → pragul „< 3" nu este satisfăcut, deci se obține nivelul inferior [dedus din „<" strict].
- Unități greșite (ppi vs dpi) → eroare de intrare.
- Categorie nesuportată → refuz cu mesaj, nu valori implicite.

## Teste de acceptanță
1. Rezoluție 300 ppi → compatibilă cu 2 sau 3 stele (valorile coincid); 400 ppi → 4 stele; 150 ppi → 1 stea; 100 ppi → sub o stea.
2. Neuniformitate 2 % → 3 stele (satisface < 3 dar nu < 1); 0,5 % → 4 stele; 9 % → sub o stea.
3. ΔE2000 mediu 6 → 2 stele (satisface < 8, nu < 5); 4,5 → 3 stele; 3,9 → 4 stele.
4. Eficiență de eșantionare 85 % → 3 stele; 92 % → 4 stele; 55 % → sub o stea.
5. MTF50 ca procent din jumătatea frecvenței de eșantionare: 50 % → 4 stele (în [40, 65]); 70 % → 3 stele (în [35, 75], în afara lui [40, 65]); 80 % → 2 stele (în [30, 85], în afara lui [35, 75]); 20 % → sub o stea.
6. Zgomot cu „>" în sursă: implementarea alege „<" și semnalează în raport că tabelul sursă diferă (decizia aprobată de un om).
7. Lipsa unei măsurători de culoare → raportul arată „nemăsurat" pentru culoare și nu degradează celelalte rânduri.

## Necunoscute
- Ediția a treia (2023/2024): praguri, categorii, metodă.
- Regula de nivel global, sensul notațiilor „+N ≤ M", paranteza de la zgomot.
- Declarații de brevet asupra metodelor de măsurare: nu s-a verificat (căutarea a întors un brevet pe caracterizarea sistemelor de imagistică, necitit).
- Licența documentului: publicație guvernamentală SUA, termeni neverificați.

## Checklist de filtru
Spec: 08-fadgi-niveluri   Data: 2026-11-09

- [x] Nicio linie de cod, pseudocod sau asamblare copiată
- [x] Niciun nume intern de funcție, variabilă sau clasă din țintă
- [x] Niciun text de UI, asset sau branding din țintă
- [x] Algoritmii sunt descriși prin intrare → ieșire, nu prin structura codului țintei
- [x] Afirmațiile sunt marcate [observat] / [dedus] / [necunoscut]
- [x] Licența/EULA țintei verificată și notată în `1-analiza/jurnal.md` (notată în `docs/clean-room/specs/JURNAL.md`; termeni neverificați)
- [ ] Specificația nu depinde de brevete sau secrete comerciale (metoda de măsurare exclusă; restul neverificat)
- [ ] Aprobată de mine înainte de predare
