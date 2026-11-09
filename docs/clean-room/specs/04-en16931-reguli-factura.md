# Funcție: Verificare a regulilor de calcul și de câmpuri obligatorii ale unei facturi electronice (model semantic EN 16931, sintaxa UBL)

Sursă publică: lista regulilor de afaceri EN 16931 pentru UBL, publicată în documentația Peppol, https://docs.peppol.eu/tdd/sk/tdd-sk/trn-tdd/rule/CEN-EN16931-UBL/ (citit 2026-11-09 prin instrument de preluare cu rezumare; pagina se oprește la regulile UBL-CR-116, deci lista poate fi incompletă). Standardul EN 16931-1 al CEN este contra cost și NU a fost citit. Regulile naționale românești (CIUS-RO) NU au fost citite.
Stare: candidat. Dimensiune: M.

## Scop
Utilizatorul are o factură XML și vrea o listă a încălcărilor de reguli (calcul, câmpuri lipsă, zecimale), formulate prin numărul regulii. Extinde verificarea existentă (care acoperă doar o parte din calcule) fără a pretinde validare Schematron completă.

## Intrări / ieșiri
- Intrare: o factură sau notă de credit UBL, deja citită în câmpuri semantice (numerotate BT în model).
- Ieșire: listă de încălcări, fiecare cu identificatorul regulii, câmpurile implicate, valoarea găsită și cea așteptată; cod de ieșire nenul dacă există cel puțin una.

## Comportament observabil
**Reguli de calcul** (parafrazate din lista regulilor) [observat]:
- BR-CO-10: totalul net al liniilor = suma sumelor nete ale liniilor.
- BR-CO-11: totalul reducerilor la nivel de document = suma reducerilor de document.
- BR-CO-12: totalul taxelor suplimentare la nivel de document = suma lor.
- BR-CO-13: total fără TVA = total linii − reduceri + taxe suplimentare de document.
- BR-CO-14: TVA total = suma TVA pe categorii, când este furnizat.
- BR-CO-15: total cu TVA = total fără TVA + TVA total, când TVA este furnizat.
- BR-CO-16: suma de plată = total cu TVA − suma achitată + suma de rotunjire.
- BR-CO-17: TVA pe categorie = bază impozabilă × cotă, rotunjit la două zecimale, când este furnizat.
- BR-CO-19/20: dacă se folosește o perioadă de facturare (la document sau la linie), se completează data de început, cea de sfârșit sau ambele.
- BR-CO-21..24: orice reducere sau taxă suplimentară (document sau linie) are motiv în text, cod de motiv sau ambele; BR-CO-05..08: codul și textul motivului descriu același lucru când sunt amândouă.
- BR-CO-03: data punctului fiscal și codul lui sunt reciproc exclusive.
- BR-CO-09: identificatorii de TVA ai vânzătorului, reprezentantului fiscal și cumpărătorului au prefix de țară (Grecia poate folosi „EL").
**Zecimale** (BR-DEC-*): sumele de reducere, taxă suplimentară, linie, totaluri, TVA, achitat, rotunjire, de plată au cel mult două zecimale; lista exactă a numerelor BR-DEC-01, 02, 05, 06, 09-20, 23-25, 27, 28 apare în sursă [observat]. Care câmp corespunde fiecărui număr: [necunoscut] (pagina dă doar denumiri generale).
**Câmpuri obligatorii** (BR-*) [observat]: identificator de specificație (BT-24, BR-01), număr factură (BT-1, BR-02), data emiterii (BT-2, BR-03), cod tip factură (BT-3, BR-04), cod monedă (BT-5, BR-05), nume cumpărător (BT-44, BR-07), țara vânzătorului (BT-40, BR-09), țara cumpărătorului (BT-55, BR-11), totalul net al liniilor (BT-106, BR-12), total fără TVA (BT-109, BR-13), suma de plată (BT-115, BR-15), cel puțin o linie (BR-16); la fiecare linie: identificator (BT-126, BR-21), cantitate (BT-129, BR-22), unitate (BT-130, BR-23), sumă netă (BT-131, BR-24), denumire articol (BT-153, BR-25), preț net (BT-146, BR-26); la reduceri/taxe: sumă și motiv (BR-31, 33, 36, 38, 41-44); la defalcarea TVA: bază impozabilă (BT-116, BR-45); la plată: cod mijloc de plată (BT-81, BR-49); reprezentant fiscal: identificator TVA (BT-63, BR-56); clasificare articol: identificator de schemă (BR-65). Regulile condiționale (ex. BR-19, 20, 48, 50, 53, 55, 61) nu au fost parcurse: [necunoscut].
**Liste de coduri** (BR-CL-*) [observat doar ca denumire]: tip document (UNTDID 1001), monede (ISO 4217 alfa-3), țări (ISO 3166-1), mijloc de plată (UNCL 4461), categorie TVA (UNCL 5305), motive reducere (UNCL 5189), motive taxă (UNCL 7161), unități (UN/ECE Rec. 20/21), scheme de identificare (ISO 6523), scheme de adresă electronică (liste CEF). Conținutul listelor nu a fost citit: [necunoscut]; verificarea valorilor din liste e în afara acestei specificații.

## Formate / protocoale
Sintaxa UBL 2.1 (căile elementelor XML pentru fiecare BT) nu a fost citită: [necunoscut]; implementarea o preia din specificația OASIS UBL 2.1 și din legarea EN 16931-UBL, obținute separat. Sumele sunt zecimale exacte; compararea se face fără virgulă mobilă binară [dedus].

## Cazuri limită și erori
- Mod de rotunjire la două zecimale pentru BR-CO-17 (în sus la jumătate, la par etc.): [necunoscut]. Toleranța la comparații (zero, o sutime): [necunoscut].
- Notă de credit: aceleași reguli de calcul; semnul sumelor [necunoscut].
- Câmp „când este furnizat" absent → regula nu se aplică [observat].
- Factură cu zero linii → BR-16.
- Fiecare regulă încălcată se raportează separat; o încălcare nu oprește verificarea celorlalte [dedus].

## Teste de acceptanță
1. O linie cu net 100.00, fără reduceri/taxe, o categorie de TVA cu bază 100.00 și cotă 19%: TVA 19.00, total fără TVA 100.00, total cu TVA 119.00, achitat 0, de plată 119.00 → zero încălcări.
2. Aceeași factură cu suma de plată 120.00 → încălcare BR-CO-16 (așteptat 119.00).
3. Două linii 10.00 + 5.50 și totalul net al liniilor declarat 15.00 → BR-CO-10 (așteptat 15.50).
4. TVA pe categorie declarat 18.99 pentru bază 100.00 și cotă 19% → BR-CO-17.
5. Sumă cu trei zecimale (12.345) într-un câmp monetar → încălcare BR-DEC (numărul depinde de câmp: [necunoscut]; testul verifică doar existența încălcării).
6. Reducere de document de 10.00 + taxă suplimentară de 5.00 pe total linii 100.00: total fără TVA 95.00 → fără încălcare; declarat 105.00 → BR-CO-13.
7. Lipsa numărului facturii → BR-02; lipsa datei → BR-03; factură fără nicio linie → BR-16.
8. Reducere fără sumă → BR-31 sau BR-41 după nivel; reducere fără motiv și fără cod de motiv → BR-CO-21 sau BR-CO-23.

## Necunoscute
- EN 16931-1 și CIUS-RO nu au fost citite; lista este cea din pagina Peppol, posibil trunchiată și parafrazată de instrumentul de preluare.
- Mod de rotunjire, toleranță, semn la notă de credit, corespondența BR-DEC ↔ câmp, calea XML a fiecărui BT.
- Termenii de reutilizare ai paginii Peppol nu au fost verificați.

## Checklist de filtru
Spec: 04-en16931-reguli-factura   Data: 2026-11-09

- [x] Nicio linie de cod, pseudocod sau asamblare copiată
- [x] Niciun nume intern de funcție, variabilă sau clasă din țintă
- [x] Niciun text de UI, asset sau branding din țintă
- [x] Algoritmii sunt descriși prin intrare → ieșire, nu prin structura codului țintei
- [x] Afirmațiile sunt marcate [observat] / [dedus] / [necunoscut]
- [x] Licența/EULA țintei verificată și notată în `1-analiza/jurnal.md` (notată în `docs/clean-room/specs/JURNAL.md`; licența paginii neverificată)
- [ ] Specificația nu depinde de brevete sau secrete comerciale (neverificat)
- [ ] Aprobată de mine înainte de predare
