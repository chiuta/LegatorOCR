# Funcție: Descoperirea dispozitivelor din rețeaua locală prin DNS-SD (parte de nume și înregistrări)

Sursă publică: IETF RFC 6763, „DNS-Based Service Discovery", S. Cheshire, M. Krochmal, februarie 2013, https://datatracker.ietf.org/doc/html/rfc6763.html (citit 2026-11-09 prin instrument de preluare cu rezumare, primele 100000 din 123829 de caractere; secțiunile 4, 5, 6, 7, 7.1, 8, 9). RFC 6762 (mDNS, transportul pe „.local") NU a fost citit: URL neaccesibil în sesiune.
Stare: candidat. Dimensiune: S.

## Scop
Utilizatorul vrea să vadă scannerele de rețea fără să tasteze adrese. Specificația acoperă numai ce se poate stabili din RFC 6763: cum arată numele de servicii, cum se citesc înregistrările de răspuns și cum se prezintă lista; tipul de serviciu al scannerelor și transportul multicast sunt în afara citirii de față.

## Intrări / ieșiri
- Intrare: tipul de serviciu căutat (parametru, ex. „_ceva._tcp") și domeniul (implicit „local."); răspunsurile primite (înregistrări PTR, SRV, TXT, adrese).
- Ieșire: listă de instanțe cu nume afișabil, gazdă, port, adrese și atribute, sau listă goală cu motiv.

## Comportament observabil
1. Numele unei instanțe: <Instanță>.<Serviciu>.<Domeniu>; înregistrările SRV și TXT au același nume [observat, 4.1].
2. Instanță: etichetă prietenoasă, UTF-8 precompus (forma de normalizare C), fără caractere de control ASCII, cel mult 63 de octeți pe etichetă; nu e limitată de regulile numelor de gazdă [observat, 4.1.1].
3. Tip de serviciu: nume cu prefix „_" urmat de „_tcp" sau „_udp"; numele are cel mult 15 caractere (litere, cifre, cratime), începe și se termină cu literă sau cifră, nu are cratime consecutive, are cel puțin o literă, fără sensibilitate la majuscule [observat, 4.1.2/7].
4. Răsfoire: interogare PTR pentru <Serviciu>.<Domeniu> returnează lista numelor de instanțe [observat, 4.1/9]. Meta-interogarea pentru tipurile prezente: „_services._dns-sd._udp.<Domeniu>" [observat, 9].
5. Rezolvare: pentru o instanță se interoghează SRV (gazdă, port, prioritate, pondere) și TXT; la mai multe SRV se respectă prioritatea și ponderea [observat, 5].
6. TXT [observat, 6]: obligatoriu pentru orice instanță, chiar gol (un singur octet zero); un TXT de lungime zero se tratează ca echivalent cu un octet zero și cu lipsa înregistrării; mărime vizată ≤ 200 de octeți, sub 400 încape în 512, sub 1300 încape într-un pachet Ethernet. Atribute „cheie=valoare" împărțite la primul „="; chei necunoscute se ignoră; cheie: cel puțin un caracter, ideal cel mult nouă, ASCII tipăribil fără „=", insensibilă la majuscule; cheie fără „=" este boolean; la chei repetate se ia prima; valorile sunt octeți opaci, pot conține „=", fără ghilimele; gazda și portul nu se repetă în TXT; un atribut „txtvers" ar trebui să fie primul.
7. Subtipuri: <subtip>._sub.<serviciu>.<domeniu> restrânge răsfoirea fără alt spațiu de nume [observat, 7.1].
8. Duplicate: aceeași instanță văzută pe mai multe interfețe sau adrese se afișează o singură dată; regula exactă a RFC-ului pentru interfețe multiple nu a fost în text: [necunoscut]; deducția este [dedus].
9. Aplicația nu trimite nimic în afara rețelei locale; în modul „offline/sigilat" al proiectului descoperirea locală rămâne permisă sau nu după decizia omului: [necunoscut].

## Formate / protocoale
Nume DNS cu etichete de cel mult 63 de octeți; TXT ca șiruri cu prefix de lungime, fiecare șir un atribut [dedus din 6.1-6.3: structura cu prefix de lungime nu a fost citită explicit]. Codarea Punycode/IDNA este încercare de rezervă la răspuns negativ, etichetă cu etichetă [observat, 4.1.3].

## Cazuri limită și erori
- Cheie repetată cu valori diferite → se folosește prima.
- TXT lipsă sau gol → instanța rămâne valabilă, fără atribute.
- Instanță cu nume cu caractere de control → respinsă.
- Răspuns SRV cu port 0 și pondere 0, prioritate 0 → substituent de nume (flagship naming), nu serviciu real; nu se afișează [observat, 8].
- Nicio instanță în timpul scurs → listă goală, nu eroare.

## Teste de acceptanță
1. „_http._tcp" este tip valid; „_nume-prea-lung-de-15._tcp" (nume peste 15 caractere) și „_a--b._tcp" (cratime consecutive) sunt invalide; „_123._tcp" invalid (fără literă).
2. Numele de instanță de 64 de octeți UTF-8 → respins; de 63 → acceptat; cu un caracter de control → respins.
3. TXT „a=1", „b", „A=2" → atribute: a = „1" (prima apariție, insensibil la majuscule), b boolean.
4. TXT cu un singur octet zero, TXT de lungime zero și lipsa TXT → același rezultat: zero atribute.
5. Valoare cu „=" în interior („k=a=b") → cheie „k", valoare „a=b".
6. SRV cu port 0 → instanța nu e afișată ca serviciu.
7. Două SRV cu priorități 10 și 20 → se alege prioritatea 10.
8. Meta-interogarea pentru domeniul „local." cere numele „_services._dns-sd._udp.local.".

## Necunoscute
- Numele de serviciu (ex. pentru scannere), cheile TXT ale dispozitivelor și regulile de transport multicast (RFC 6762) nu au fost citite.
- Comportamentul pe mai multe interfețe și textul exact al secțiunilor 8-9 dincolo de rezumat.
- Prezența în registrul IANA de nume de servicii nu a fost verificată.

## Checklist de filtru
Spec: 07-dnssd-descoperire   Data: 2026-11-09

- [x] Nicio linie de cod, pseudocod sau asamblare copiată
- [x] Niciun nume intern de funcție, variabilă sau clasă din țintă
- [x] Niciun text de UI, asset sau branding din țintă
- [x] Algoritmii sunt descriși prin intrare → ieșire, nu prin structura codului țintei
- [x] Afirmațiile sunt marcate [observat] / [dedus] / [necunoscut]
- [x] Licența/EULA țintei verificată și notată în `1-analiza/jurnal.md` (notată în `docs/clean-room/specs/JURNAL.md`)
- [ ] Specificația nu depinde de brevete sau secrete comerciale (neverificat)
- [ ] Aprobată de mine înainte de predare
