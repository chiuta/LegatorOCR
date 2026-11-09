# Funcție: Extinderea unei semnături PDF cu material de validare pe termen lung (nivelurile B-LT și B-LTA)

Sursă publică: ETSI EN 319 142-1, „Electronic Signatures and Infrastructures (ESI); PAdES digital signatures; Part 1: Building blocks and PAdES baseline signatures", copie publică la https://developer.adobe.com/document-services/docs/assets/70fef5ab21077e8ef86384a148080a0d/ETSI_EN_319_142-1.pdf (copie găzduită de terț; versiunea exactă nu a fost stabilită). Citit 2026-11-09 prin instrument de preluare cu rezumare; clauzele 5.3, 5.4.2, 5.4.3, 6.1, 6.3, tabelul 1.
Stare: candidat. Dimensiune: L.

## Scop
Programul semnează deja PDF-uri (semnătură CMS detașată, cu ștampilă de timp pe semnătură). Utilizatorul vrea ca semnătura să poată fi verificată și peste ani, când certificatele au expirat: se încorporează dovezile de validare (nivelul „LT"), apoi se pune o ștampilă de timp pe tot documentul (nivelul „LTA"), reînnoibilă.

## Intrări / ieșiri
- Intrare: un PDF cu cel puțin o semnătură; (pentru LT) certificatele lanțului și valorile de revocare ale semnatarului, ale autorității de ștampilare și ale celorlalte semnături; (pentru LTA) un răspuns de ștampilă de timp RFC 3161 pentru acoperirea documentului.
- Ieșire: același PDF, cu octeții anteriori nemodificați, plus actualizări incrementale; raport de verificare cu nivelul atins.

## Comportament observabil
**Niveluri** [observat, 6.1, 6.3, tabel 1]: B-B = atribute semnate la creare; B-T = timp de încredere (atribut de ștampilă pe semnătură sau ștampilă de document); B-LT = B-T plus dicționar de validare în document cu certificate și valori de revocare; B-LTA = B-LT plus una sau mai multe ștampile de document aplicate după ce tot materialul de validare lipsă a fost inclus.
**Dicționarul de semnătură** [observat, 5.3, 6.3]: câmpul de conținut este o structură CMS detașată codificată DER, în hexazecimal; un singur semnatar; subfiltrul este ETSI.CAdES.detached; ByteRange acoperă tot fișierul în afara valorii câmpului de conținut; spațiul se rezervă înainte de calculul rezumatului; codificările PKCS#1 și PKCS#7 din ISO 32000-1 nu se folosesc; intrarea Cert este interzisă; MD5 este interzis; atributul CMS de timp de semnare este interzis (momentul pretins se scrie în intrarea M a dicționarului); atributul de certificat de semnare versiunea 2 este preferat celui din versiunea 1; atributul de indicare a tipului de angajament nu se combină cu intrarea Reason, iar Reason nu se combină cu atributul de politică de semnare.
**Dicționarul de validare (DSS)** [observat, 5.4.2-5.4.3]: referit din catalogul documentului; conține tablouri la nivel de document cu referințe indirecte la fluxuri pentru certificate, răspunsuri OCSP și liste CRL, în cod DER; dicționarul VRI e opțional și, după clauza 6.3, nu ar trebui folosit; dacă există, cheia fiecărei intrări este rezumatul SHA-1 (hexazecimal, majuscule) al valorii de conținut a semnăturii și poate referi doar valori din DSS. DSS și VRI se pun în secțiuni de actualizare incrementală; actualizarea VRI vine după actualizarea semnăturii la care se referă; la actualizarea DSS se păstrează valorile din DSS anterior. Restricțiile DocMDP nu blochează actualizările care adaugă DSS, VRI, certificate, CRL sau OCSP.
**Ștampila de document** [observat, 5.4.3]: dicționar de semnătură cu Type DocTimeStamp și SubFilter ETSI.RFC3161; conținutul este un token RFC 3161 în hexazecimal, al cărui rezumat acoperă octeții din ByteRange; ByteRange acoperă tot documentul inclusiv dicționarul ștampilei, dar nu token-ul; intrările Cert, Reference, Changes, R, Prop_AuthTime, Prop_AuthType sunt interzise, iar Name, M, Location, Reason, ContactInfo nu ar trebui să existe; dacă V există, este 0.
**Ordinea actualizărilor** [observat, 5.4.2, 6.3]: LT: semnătura, apoi o actualizare cu DSS (opțional VRI). LTA: după DSS, o actualizare cu ștampila de document; înainte de ea, tot materialul de validare pentru semnatar, certificatele de atribut și certificatele autorității de ștampilare e deja în DSS; ștampile ulterioare se pot adăuga mai târziu, fiecare precedată de materialul nou necesar.
**Verificare** (în intrare): programul poate afișa nivelul observat al fiecărei semnături: B-B, B-T (token pe semnătură sau ștampilă de document), B-LT (DSS conține ce cere semnătura), B-LTA (ștampilă de document peste DSS). Metodele de validare a lanțului și ale revocării (RFC 5280, RFC 6960) nu au fost citite: [necunoscut].

## Cazuri limită și erori
- Mărimea spațiului rezervat pentru conținut: nu e dată numeric; trebuie ales și dacă nu încape token-ul, operația eșuează înainte de scriere [observat: standardul cere doar rezervarea; dedus eșecul].
- Valori de revocare indisponibile (fără rețea sau regim sigilat): se refuză trecerea la LT cu mesaj, nu se produce un LT fals [dedus]. Obținerea valorilor (OCSP, CRL) este [necunoscut].
- Fișier cu semnături multiple: DSS la nivel de document cuprinde valorile tuturor; verificarea cere ca fiecare să fie acoperită [dedus].
- Semnătură cu algoritm de rezumat interzis (MD5) → respinsă la verificare [observat].
- Fișier cu actualizări ulterioare ale ștampilei: se avertizează conținutul adăugat după ea [dedus].

## Teste de acceptanță
1. După LT, octeții fișierului dinainte de operație sunt identici cu începutul fișierului rezultat (actualizare incrementală).
2. Catalogul rezultat conține DSS cu tablouri de referințe la fluxuri; fiecare flux se parsează ca DER (certificat, OCSP sau CRL).
3. Dacă VRI este scris, fiecare cheie are 40 de caractere hexazecimale majuscule și este SHA-1 al conținutului semnăturii corespunzătoare; fiecare valoare referită există în DSS.
4. După LTA, ultimul obiect de semnătură are Type DocTimeStamp și SubFilter ETSI.RFC3161, fără intrările interzise; ByteRange acoperă tot fișierul în afara conținutului; rezumatul din token este cel al acelor octeți.
5. Ordinea octeților în fișier: semnătură înainte de DSS înainte de ștampilă.
6. Reluare LTA a unui fișier deja LTA: noul DSS păstrează valorile vechi și se adaugă o a doua ștampilă.
7. Semnătură cu atribut CMS de timp de semnare sau cu Cert → respinsă la verificarea de conformitate.
8. Fără material de revocare → refuz clar, fișierul rămâne neschimbat.

## Necunoscute
- Versiunea exactă a standardului (copie găzduită de terț, fără dată) și diferențele față de ediția oficială ETSI.
- Cum se obțin și se validează lanțurile și valorile de revocare; regulile de reînnoire a ștampilei; mărimea rezervată.
- Declarații de brevet: standardul ETSI are politică IPR; nu a fost consultată baza de date.
- Termenii de reutilizare ETSI și ai găzduirii.

## Checklist de filtru
Spec: 09-pades-lt-lta   Data: 2026-11-09

- [x] Nicio linie de cod, pseudocod sau asamblare copiată
- [x] Niciun nume intern de funcție, variabilă sau clasă din țintă
- [x] Niciun text de UI, asset sau branding din țintă
- [x] Algoritmii sunt descriși prin intrare → ieșire, nu prin structura codului țintei
- [x] Afirmațiile sunt marcate [observat] / [dedus] / [necunoscut]
- [x] Licența/EULA țintei verificată și notată în `1-analiza/jurnal.md` (notată în `docs/clean-room/specs/JURNAL.md`; termeni neverificați)
- [ ] Specificația nu depinde de brevete sau secrete comerciale (IPR ETSI neverificat)
- [ ] Aprobată de mine înainte de predare
