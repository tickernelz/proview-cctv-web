import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const S = (n) => `../shots/${n}.jpg`;
const pages = [];
let pn = 0;

const run = (n, t) => `<div class="run"><span>PROVIEW · redesign v1 · ${t}</span><span>${n}</span></div>`;
const page = (title, body, opts = {}) => {
  pn++;
  pages.push(`<section class="pg${opts.dark ? ' dark' : ''}">${body}${run(String(pn).padStart(2, '0'), title)}</section>`);
};
const head = (title, where) => `<div class="head"><h2>${title}</h2><span class="where">${where}</span></div>`;
const one = (title, where, shot, notes, opts = {}) =>
  page(title, `${head(title, where)}<div class="split"><div><img class="shot" src="${S(shot)}" alt=""></div><div class="notes">${notes}</div></div>`, opts);
const fig = (src, cap) => `<figure style="margin:0"><img class="shot" src="${src}" alt=""><figcaption class="cap">${cap}</figcaption></figure>`;
const two = (title, where, a, ca, b, cb, foot = '', opts = {}) =>
  page(title, `${head(title, where)}<div class="grid2">${fig(S(a), ca)}${fig(S(b), cb)}</div>${foot ? `<div class="notes" style="margin-top:4mm">${foot}</div>` : ''}`, opts);
const sw = (hex) => `<span class="sw" style="background:${hex}"></span>`;
const kbd = (k) => `<kbd>${k}</kbd>`;

page('Sampul', `
<p class="mono mute" style="margin-bottom:10mm">ZHA-3941 · 27 Oktober 2026 · tahap mockup, belum ada perubahan kode</p>
<h1 style="font-size:34pt;max-width:210mm">PROVIEW di meja kerja: gambarnya yang jadi halaman</h1>
<p class="lede">Usulan desain ulang klien desktop/web PROVIEW. Tiga arah dibandingkan, satu direkomendasikan, lalu arah itu digambar penuh: semua layar dan keadaan di 1440×900 dan 1280×800, tema gelap dan terang, plus sistem visual, catatan interaksi, aksesibilitas dengan angka kontras terhitung, dan daftar hal yang sengaja tidak dipakai.</p>
<div style="margin-top:auto;display:grid;grid-template-columns:1fr 1fr;gap:10mm">
<div class="toc">
<span>02</span><span>Apa yang salah di UI sekarang (diukur)</span><span>02–03</span>
<span>A</span><span>Arah A — Rak</span><span>04–05</span>
<span>B</span><span>Arah B — Jendela (direkomendasikan)</span><span>06–07</span>
<span>C</span><span>Arah C — Dinding</span><span>08–09</span>
<span>10</span><span>Perbandingan dan rekomendasi</span><span>10</span>
<span>11</span><span>Live, kamera, lensa, gerak, layar penuh, interkom, rekam</span><span>11–19</span>
</div>
<div class="toc">
<span>20</span><span>Grid 2–4 kamera dan kegagalannya</span><span>20–22</span>
<span>23</span><span>Memuat, gagal, beku, offline</span><span>23–25</span>
<span>26</span><span>Masuk, galat, sesi habis</span><span>26–28</span>
<span>29</span><span>Pengaturan dan info perangkat</span><span>29–31</span>
<span>32</span><span>Sistem: tipe, warna, ruang, ikon, gerak, komponen</span><span>32–35</span>
<span>36</span><span>Interaksi, aksesibilitas, anti-slop, kritik diri, implementasi</span><span>36–42</span>
</div></div>`);

page('Kritik UI sekarang', `${head('Apa yang salah di UI sekarang', 'web/ @ 43b1879 · fixture provider · 1440×900')}
<div class="split"><div><img class="shot" src="../img/old-live.jpg" alt=""><p class="cap">Layar Live sekarang, dijalankan lokal dengan <span class="mono">PROVIEW_PROVIDER=fixture</span>. Angka di kanan diukur dari DOM halaman ini, bukan ditaksir.</p></div>
<div class="notes">
<h3>Angka</h3>
<dl class="kv">
<dt>gambar</dt><dd>Stage terukur 796×260 px saat dibuka: 16 % layar. Bahkan diisi gambar 16:9 selebar penuh, paling besar 796×448: <b>27,5 %</b>. Sisanya chrome.</dd>
<dt>kontrol</dt><dd><b>27</b> kontrol terlihat di layar Live, <b>16</b> di antaranya nonaktif.</dd>
<dt>eyebrow</dt><dd>5 label kapital ber-tracking di satu layar: VIEW, QUALITY, PAN AND TILT, DEVICE ACTIONS, IN THIS BROWSER.</dd>
<dt>radius</dt><dd>5 radius berbeda (6, 10, 14, 999 px, 50 %).</dd>
<dt>merah</dt><dd>Satu merah untuk tiga arti: perangkat terpilih (baris pink), tombol Mute, dan "Stream failed".</dd>
</dl>
<h3>Kenapa terasa "AI slop"</h3>
<ul>
<li>Semua diberi bobot yang sama: tombol PTZ, tujuh aksi perangkat, penjelasan lensa, dan video berebut perhatian.</li>
<li>Panel kanan penuh tombol yang sebagian besar tidak bisa dipakai. Kejujuran ("diblokir, ini buktinya") benar, tapi tempatnya bukan di layar utama.</li>
<li>Penjelasan teknis ("Why the two lens views differ") tinggal permanen di layar yang dibuka tiap hari.</li>
<li>Email akun, tombol tema berupa teks, dan tombol Watch di baris yang sudah terpilih: kontrol yang tidak mengubah apa yang dilihat.</li>
</ul></div></div>`);

page('Kritik UI sekarang', `${head('Masuk dan Pengaturan sekarang', 'web/index.html 361 baris · style.css 593 baris')}
<div class="grid2">${fig('../img/old-login.jpg', '<b>Masuk.</b> Kolom tengah, judul besar, satu CTA: pola hero-di-tengah. Lede menjelaskan cookie untuk developer, bukan untuk pemilik kamera.')}
${fig('../img/old-settings.jpg', '<b>Pengaturan.</b> Satu halaman panjang, lima kelompok berbingkai dengan bobot sama. Tidak ada tempat yang menunjukkan ada perubahan belum tersimpan.')}</div>
<div class="notes" style="margin-top:5mm;display:grid;grid-template-columns:1fr 1fr;gap:10mm">
<div><h3>Yang dipertahankan</h3><p>Isinya jujur dan tetap: empat keadaan stage (memuat, main, beku, gagal), "playing" baru setelah frame terdekode, laporan kapabilitas dengan bukti dari perangkat, label "needs restart" di field, rahasia tidak pernah dibaca balik ke halaman, pengaturan bisa dibuka sebelum masuk.</p></div>
<div><h3>Yang dibuang</h3><p>Rail perangkat permanen, panel Camera controls, segmented View/Quality di atas stage, eyebrow kapital, badge sumber di atas video, disclosure penjelasan lensa di layar Live, tombol teks Theme, email akun di topbar, font sistem yang berbeda di tiap OS.</p></div></div>`);

page('Arah A — Rak', `${head('Arah A — Rak', 'aplikasi NVR konsumen yang dirapikan · terang')}
<div class="split"><div><img class="shot" src="${S('dir-a')}" alt=""></div>
<div class="notes"><h3>Gagasan</h3><p>Rail kiri dengan thumbnail tiap kamera, video besar di kanan, tombol pil di bawah. Bentuk yang dikenal dari aplikasi XMEye/Ezviz di desktop.</p>
<h3>Yang bagus</h3><ul><li>Kamera lain selalu terlihat; pindah kamera satu klik.</li><li>Paling mudah dipahami tanpa penjelasan.</li></ul>
<h3>Ongkosnya</h3><ul><li>Gambar hanya <b>54,4 %</b> layar (diukur). 264 px rail dibayar oleh video setiap saat, padahal pengguna punya satu sampai lima kamera.</li><li>Thumbnail bergerak berarti lima stream terbuka; thumbnail diam berarti gambar basi tanpa jam. Yang pertama memakan bandwidth, yang kedua berbohong tentang "live".</li><li>Pil berbayang, badge hijau "Live", rail putih: dekat sekali ke template dashboard.</li></ul></div></div>`);

page('Arah A — Rak', `${head('Arah A — kenapa tidak', 'dinilai terhadap PRODUCT.md')}
<div class="notes cols" style="font-size:10pt">
<div><h3>Prinsip 1 · gambar menang</h3><p>Gagal. Rail menetap memakan 18 % lebar di 1440 px dan 21 % di 1280 px. Di laptop 1280×800 video turun mendekati setengah layar.</p></div>
<div><h3>Prinsip 2 · warna hanya untuk keadaan</h3><p>Lulus setelah diturunkan ke netral seperti di mock ini. Versi awalnya memakai gradien biru pada logo dan tombol utama; itu dicabut karena dekoratif.</p></div>
<div><h3>Prinsip 3 · katakan yang benar</h3><p>Thumbnail adalah masalahnya: stream per baris melanggar "jendela tetap responsif dengan empat tile"; snapshot statis harus diberi jam, dan baris jadi ramai.</p></div>
<div><h3>Kapan arah ini menang</h3><p>Pengguna dengan 8–16 kanal yang pindah kamera terus-menerus. Itu bukan pengguna di PRODUCT.md, yang membuka satu kamera, melihat sebentar, lalu menutup.</p></div>
<div><h3>Yang diambil ke arah B</h3><p>Pindah kamera harus satu langkah: di B menjadi menu di nama kamera plus tombol angka ${kbd('1')}–${kbd('9')}, tanpa menempati layar.</p></div>
</div>`);

page('Arah B — Jendela', `${head('Arah B — Jendela', 'direkomendasikan · gelap · 1440×900')}
<div class="split"><div><img class="shot" src="${S('live-dark')}" alt=""></div>
<div class="notes"><h3>Gagasan</h3><p>Aplikasi ini satu jendela ke satu kamera. Video mengisi halaman; di atasnya satu baris (nama kamera yang juga pemilih kamera, tiga ikon), di bawahnya satu baris keterangan dan satu baris kontrol. Tidak ada panel samping.</p>
<h3>Angka</h3><dl class="kv"><dt>gambar</dt><dd><b>72,3 %</b> layar (diukur di 1440×900), naik dari maksimum 27,5 %.</dd><dt>kontrol</dt><dd>11 terlihat, semuanya aktif. Kontrol perangkat yang tidak didukung tidak tampil di sini; tabel buktinya ada di Pengaturan.</dd></dl>
<h3>Rasa</h3><p>Grafit netral yang tidak bersaing dengan gambar, tipe teknis yang tenang (IBM Plex), satu baris teks kecil yang mengatakan apa yang sedang terjadi. Seperti pemutar video yang baik, bukan ruang kontrol.</p></div></div>`);

page('Arah B — Jendela', `${head('Arah B — dengan beberapa kamera', 'grid · terang · 1440×900')}
<div class="split"><div><img class="shot" src="${S('grid-light')}" alt=""></div>
<div class="notes"><h3>Satu kamera dulu, grid bila diminta</h3><p>Layar awal selalu satu kamera, karena itu pekerjaan utamanya. Grid satu klik (atau ${kbd('G')}) untuk "semua baik-baik saja?" dan menampilkan seluruh frame tiap kamera, tanpa crop.</p>
<h3>Gagal per tile</h3><p>Tiap tile streamnya sendiri dan gagalnya sendiri; tile lain tetap main. Keterangan di bawah tile, bukan di atas gambar.</p>
<h3>Satu bahasa</h3><p>Keterangan, jam monospace, dot status, dan baris kontrol di grid sama persis dengan layar satu kamera. Tile terpilih ditandai garis tinta 1,5 px, bukan warna aksen.</p></div></div>`);

page('Arah C — Dinding', `${head('Arah C — Dinding', 'monitor ruang kontrol · gelap pekat')}
<div class="split"><div><img class="shot" src="${S('dir-c')}" alt=""></div>
<div class="notes"><h3>Gagasan</h3><p>Grid penuh tepi ke tepi, rail ikon 54 px, label kanal monospace di atas gambar, bitrate dan resolusi di setiap tile. Bahasa visual software NVR profesional.</p>
<h3>Yang bagus</h3><ul><li>Video menutup hampir seluruh layar.</li><li>Cocok untuk monitor kedua yang dibiarkan menyala.</li></ul>
<h3>Ongkosnya</h3><ul><li>Gambar di-crop (<span class="mono">object-fit: cover</span>) supaya grid rapat: lensa ganda kehilangan sebagian, bertentangan dengan aturan "grid menampilkan seluruh frame".</li><li>Label di atas video selalu berebut dengan gambar, terutama gambar terang.</li><li>Kode kanal, bitrate, ikon tanpa teks: harus dipelajari. PRODUCT.md menyebut ini anti-referensi.</li></ul></div></div>`, { dark: true });

page('Arah C — Dinding', `${head('Arah C — kenapa tidak', 'dinilai terhadap PRODUCT.md')}
<div class="notes cols" style="font-size:10pt">
<div><h3>Pemakainya salah</h3><p>C dibuat untuk operator yang menatap layar seharian. Pengguna PROVIEW membuka aplikasi, melihat sebentar, lalu menutup. "Rasanya harus seperti alat rumah tangga, bukan ruang kontrol."</p></div>
<div><h3>Refleks kategori</h3><p>"Aplikasi CCTV → gelap, monospace, label hijau terminal" adalah tebakan pertama dari kategorinya saja: tanda slop tingkat pertama menurut skill impeccable.</p></div>
<div><h3>Ikon tanpa teks</h3><p>Rail ikon tanpa label membuat pengguna menebak. B memberi teks pada setiap kontrol yang mengubah gambar; hanya tiga ikon navigasi dan layar penuh yang tanpa teks, dengan nama aksesibel dan tooltip.</p></div>
<div><h3>Yang diambil ke arah B</h3><p>Layar penuh B adalah sepotong C: tepi ke tepi, satu baris monospace kecil di bawah, tidak ada chrome lain. Bedanya, di B itu pilihan, bukan default.</p></div>
</div>`, { dark: true });

page('Rekomendasi', `${head('Rekomendasi: Arah B — Jendela', 'angka gambar diukur dari DOM mock di 1440×900')}
<table style="margin-bottom:6mm"><thead><tr><th style="width:52mm">Kriteria</th><th>A · Rak</th><th>B · Jendela</th><th>C · Dinding</th></tr></thead><tbody>
<tr><td>Luas gambar, 1 kamera</td><td class="num">54,4 %</td><td class="num"><b>72,3 %</b></td><td>tidak ada mode 1 kamera</td></tr>
<tr><td>Seluruh frame terlihat</td><td>ya</td><td>ya</td><td>tidak (crop)</td></tr>
<tr><td>Kontrol di layar utama</td><td>8 tombol + 5 baris kamera</td><td>11, semua aktif</td><td>4 ikon tanpa teks</td></tr>
<tr><td>Stream terbuka saat menonton 1 kamera</td><td>1 + 4 thumbnail</td><td>1</td><td>4</td></tr>
<tr><td>Perlu belajar sebelum pakai</td><td>tidak</td><td>tidak</td><td>ya</td></tr>
<tr><td>Laptop 1280×800</td><td>video mengecil karena rail</td><td>struktur sama</td><td>struktur sama</td></tr>
<tr><td>JS tambahan dari sekarang</td><td>thumbnail + rail</td><td>menu kamera, tahan-untuk-bicara</td><td>grid saja</td></tr>
</tbody></table>
<div class="notes cols"><div><h3>Alasan</h3><p>B satu-satunya arah yang membuat gambar benar-benar elemen terbesar dan paling terang, tanpa memotong frame dan tanpa membuka stream yang tidak ditonton. Ia juga yang paling sedikit menuntut pengguna belajar: satu nama kamera di atas, satu baris kontrol berlabel di bawah.</p></div>
<div><h3>Risiko yang diterima</h3><p>Kamera lain tidak terlihat terus-menerus. Ditebus dengan grid satu klik, tombol angka, dot status di menu kamera, dan banner bila kamera yang sedang dibuka tidak menjawab.</p></div></div>`);

one('Live — anatomi', 'Gate camera · dual lens · gelap · 1440×900', 'live-dark', `
<h3>Urutan baca</h3><ol style="margin:0 0 2mm;padding-left:4mm"><li><b>Gambar.</b> Satu-satunya elemen terang; stage hitam di kedua tema.</li><li><b>Nama kamera</b> kiri atas, dengan dot status.</li><li><b>Keterangan</b> di bawah gambar: nama, keadaan dalam kata-kata, jam.</li><li><b>Kontrol</b>: aksi di kiri, tampilan di kanan.</li></ol>
<h3>Kenapa tiap elemen ada</h3><dl class="kv">
<dt>nama ▾</dt><dd>Menjawab "kamera mana" dan sekaligus pemilih kamera. Hapus → butuh rail.</dd>
<dt>3 ikon</dt><dd>Grid, Pengaturan, Tema. Tidak mengubah gambar, jadi kecil dan tanpa teks.</dd>
<dt>keterangan</dt><dd>"live, bottom lens, 0.4 s behind" satu-satunya bukti gambar ini terkini. Hapus → beku tak terlihat.</dd>
<dt>Talk, Sound</dt><dd>Suara keluar dan masuk, dikelompokkan.</dd>
<dt>Record, Snapshot</dt><dd>Menyimpan sesuatu, dikelompokkan.</dd>
<dt>Move</dt><dd>Hanya muncul untuk lensa yang bisa digerakkan.</dd>
<dt>lensa, kualitas</dt><dd>Mengubah tampilan, bukan kamera; teks kecil tanpa bingkai.</dd>
<dt>⤢</dt><dd>Layar penuh.</dd></dl>`);

page('Live — terang', `${head('Live — tema terang', '1440×900 · stage tetap hitam')}
<div class="full"><img class="shot" src="${S('live-light')}" alt=""></div>
<p class="cap" style="margin-top:2mm;max-width:none">Tema terang bukan inversi: halaman abu pucat (bukan putih, bukan krem) supaya video tetap paling terang tanpa ruangan terasa menyilaukan. Stage dan teks di atasnya memakai token <span class="mono">--stage-*</span> yang sama di kedua tema.</p>`);

two('Laptop 1280×800', 'live · gelap dan terang', 'live-1280-dark', '<b>Gelap.</b> Struktur sama, tanpa breakpoint: topbar 56 px, keterangan satu baris, kontrol 64 px; sisanya video.', 'live-1280-light', '<b>Terang.</b> Di tinggi 800 px gambar 16:9 masih mengisi lebar penuh.',
  '<p>Di bawah 1100 px lebar label Lens dan Quality pindah ke menu ⋯; di bawah 760 px Talk/Sound/Record/Snapshot menjadi ikon dengan tooltip. Tidak ada tipe fluid.</p>');

two('Satu perangkat dan belum ada perangkat', 'gelap · terang · 1440×900', 'live-single-dark', '<b>Satu kamera.</b> Nama tanpa ▾ karena tidak ada yang dipilih; ikon grid nonaktif. Kamera berlensa tunggal tidak punya pemilih lensa atau Move.', 'devices-empty-light', '<b>Belum ada kamera.</b> Kalimat yang mengajari langkah berikutnya, dua aksi, tanpa ilustrasi.',
  '<p><b>Interaksi.</b> Refresh memanggil ulang daftar perangkat dan menampilkan skeleton nama kamera; bila tetap kosong, kalimat yang sama muncul lagi dengan jam pengecekan terakhir.</p>');

one('Pemilih kamera', 'menu dari nama kamera · gelap', 'live-switcher-dark', `
<h3>Klik</h3><p>Nama kamera membuka menu di bawahnya. Tiap baris: dot status, nama, kanal dan jenis lensa, angka pintas. Kamera offline tetap ada di daftar dengan jamnya.</p>
<h3>Keyboard</h3><p>${kbd('1')}–${kbd('9')} pindah kamera dari mana saja. Di menu: ${kbd('↑')}${kbd('↓')} pindah, ${kbd('Enter')} pilih, ${kbd('Esc')} tutup dan fokus kembali ke nama.</p>
<h3>Pindah kamera</h3><p>Stream lama ditutup setelah stream baru menampilkan frame pertama; sampai itu frame lama tetap tampil dengan keterangan "connecting to Front door". Tidak ada layar hitam di antaranya.</p>
<h3>JS</h3><p>Popover API (<span class="mono">popovertarget</span>) bila WebView mendukung; kalau tidak, <span class="mono">position: fixed</span> + ±40 baris untuk fokus berputar.</p>`);

two('Pemilih lensa', 'Gate camera · lensa ganda', 'lens-menu-light', '<b>Menu lensa.</b> "Both lenses" menampilkan frame seperti dikirim kamera: dua gambar bertumpuk. Kalimat di bawah menyatakan pembagian disimpulkan dari bentuk frame.', 'lenses-dark', '<b>Lensa atas dipilih.</b> Crop dari frame yang sama, tanpa koneksi kedua. Lensa atas tetap, jadi Move hilang dari baris kontrol.',
  `<p><b>Interaksi.</b> ${kbd('L')} memutar Both → Top → Bottom. Label lensa memakai peran ("pan and tilt from here", "fixed, aimed by hand") hanya bila server melaporkan PTZ; sebelum itu posisi. Penjelasan panjang di UI lama menjadi satu kalimat di menu ini.</p>`);

one('Gerak (pan/tilt)', 'lensa bawah · Move aktif · gelap', 'ptz-dark', `
<h3>Kapan ada</h3><p>Tombol Move tampil hanya bila laporan kapabilitas menyebut PTZ <i>supported</i> untuk lensa yang sedang ditampilkan. Lensa tetap tidak pernah menampilkan pad.</p>
<h3>Pad</h3><p>Pojok kanan bawah gambar, di atas latar gelap 90 % agar terbaca di gambar apa pun. Tombol 40×40 px. Daftar preset di bawahnya, nama dari perangkat.</p>
<h3>Interaksi</h3><p>Tahan tombol arah atau ${kbd('←')}${kbd('↑')}${kbd('→')}${kbd('↓')}: kamera bergerak selama ditahan (langkah 300 ms sesuai laporan server). ${kbd('Space')} berhenti. ${kbd('Esc')} atau Move lagi menutup pad. Keterangan menulis lensa dan backend yang benar-benar dipakai server.</p>
<h3>Gagal</h3><p>Penolakan server muncul di keterangan selama 6 detik, bukan toast.</p>`);

one('Layar penuh', 'F · gelap', 'fullscreen-dark', `
<h3>Yang tersisa</h3><p>Hanya gambar dan satu baris monospace kecil: kamera, lensa, live, jam. Baris itu tetap karena di layar penuh pun gambar beku harus bisa dibedakan dari gambar live.</p>
<h3>Interaksi</h3><p>${kbd('F')} atau ⤢ masuk; ${kbd('F')} atau ${kbd('Esc')} keluar. Gerak mouse menampilkan baris kontrol yang sama di bawah gambar selama 2 detik; pintasan keyboard tetap jalan tanpa memunculkannya.</p>
<h3>Target</h3><p>Elemen stage, bukan halaman, lewat Fullscreen API; di desktop Tauri juga jendela penuh.</p>`, { dark: true });

two('Interkom dan rekam', 'tahan untuk bicara · rekam berjalan', 'talk-dark', '<b>Bicara.</b> Talk menjadi tinta penuh selama ditahan, dengan meter level mikrofon yang nyata. Keterangan: "the microphone is open"; kanan: "release to stop".', 'record-light', '<b>Rekam.</b> Satu-satunya warna aksen di layar: tombol berlatar merah lembut dengan timer, dan "Recording 00:42" di keterangan.',
  `<p><b>Interaksi.</b> Talk: tahan klik atau tahan ${kbd('T')}; lepas berhenti. Bukan toggle, supaya mikrofon tidak tertinggal terbuka. Bila perangkat tidak punya speaker, Talk tidak tampil dan Pengaturan → This camera menyebut alasannya. Sound (${kbd('M')}) toggle biasa. Record (${kbd('R')}) klik lagi berhenti dan nama file tampil di keterangan 6 detik. Snapshot (${kbd('S')}) sama.</p>`);

one('Grid 4 kamera', 'gelap · 1440×900', 'grid-dark', `
<h3>Tata letak</h3><p>2×2 untuk 3–4 kamera, 2×1 untuk 2. Tile memakai rasio gambar sendiri; sisa ruang adalah stage hitam, bukan crop.</p>
<h3>Interaksi</h3><p>Klik tile memilihnya (garis tinta). Klik ganda atau ${kbd('Enter')} membuka kamera itu di Live. ${kbd('←')}${kbd('→')}${kbd('↑')}${kbd('↓')} pindah tile. ${kbd('G')} kembali ke Live pada tile terpilih.</p>
<h3>Suara</h3><p>Grid selalu bisu; tidak ada Talk atau Record di sini.</p>
<h3>Ringkasan</h3><p>"4 cameras · 3 live · 1 offline" di baris bawah: satu kalimat yang menggantikan kartu statistik.</p>`, { dark: true });

two('Grid 2 kamera dan tile gagal', 'terang', 'grid2-light', '<b>Dua kamera.</b> Satu baris, tinggi penuh.', 'grid-fail-light', '<b>Satu tile gagal.</b> Kalimat penolakan dari server di tile itu sendiri, "Try again" untuk tile itu saja. Tile lain tetap main.');

two('Grid — gelap dan 1280', 'gagal + offline · laptop', 'grid-fail-dark', '<b>Gagal + offline</b> di tema gelap. Teks di stage memakai token stage, sama di kedua tema.', 'grid-1280-light', '<b>1280×800.</b> Grid tetap 2×2; tile mengecil, keterangan tetap satu baris.');

two('Memuat', 'skeleton saat aplikasi dibuka · stage saat stream diminta', 'boot-light', '<b>Membuka aplikasi.</b> Skeleton mengikuti bentuk layar Live yang akan datang (nama, keterangan, baris kontrol), bukan spinner di tengah. Stage langsung hitam.', 'live-loading-dark', '<b>Meminta stream.</b> Garis 2 px di tepi atas stage dan satu kalimat apa yang ditunggu. Keterangan: "connecting — nothing has been received yet".',
  '<p>Skeleton tidak berkilau. Garis memuat bergerak 1,2 s linear; dengan <span class="mono">prefers-reduced-motion</span> ia diam di 30 % dan kalimatnya tetap. Setelah 6 detik tanpa frame yang bisa didekode, keadaan menjadi gagal.</p>');

two('Gagal dan beku', 'stream gagal · gambar berhenti', 'live-fail-light', '<b>Gagal.</b> Judul, jawaban server apa adanya dalam monospace, dua aksi. Tanpa ikon besar.', 'live-stalled-dark', '<b>Beku.</b> Frame terakhir tetap tampil karena ia nyata; keterangan berubah kuning: "frozen 12 s — the frame below is from 21:39:58". Snapshot tetap aktif.',
  '<p>"Try again" meminta tiket baru dan kembali ke keadaan memuat. "Check the camera password" membuka Pengaturan → Configuration dengan field itu terfokus.</p>');

two('Offline', 'kamera tidak menjawab · komputer tanpa jaringan', 'live-offline-dark', '<b>Kamera offline.</b> Banner tipis di atas, satu kalimat penyebab, "Retry now". Stage menyebut frame terakhir dan bahwa gambar kembali sendiri.', 'nonetwork-light', '<b>Komputer offline.</b> Banner yang sama, kalimat berbeda. Gambar terakhir tetap tampil, ditandai "stale".',
  '<p>Banner hanya untuk keadaan yang memengaruhi seluruh jendela atau kamera yang sedang dibuka. Kamera lain yang offline muncul sebagai dot abu di menu kamera.</p>');

one('Masuk', 'gelap · 1440×900', 'signin-dark', `
<h3>Bentuk</h3><p>Form di kiri, stage kosong 16:9 di kanan: tempat gambar akan muncul. Sejak detik pertama terlihat bahwa ini aplikasi untuk satu gambar.</p>
<h3>Isi</h3><p>Judul menyatakan pekerjaannya. Lede dua kalimat: akun sama dengan di HP, kata sandi kamera tetap di mesin ini. Tab Account / This network. Satu tombol tinta penuh.</p>
<h3>Interaksi</h3><p>Fokus awal di Email. ${kbd('Enter')} mengirim. Saat mengirim, label tombol menjadi "Signing in…" dengan lebar tetap. Tab "This network" nonaktif dengan kalimat penjelas bila server tidak memakai provider lokal.</p>
<h3>Pengaturan</h3><p>Ikon kanan atas dan kalimat di bawah form membuka Pengaturan sebelum masuk.</p>`, { dark: true });

two('Galat masuk dan sesi habis', 'terang · gelap', 'signin-error-light', '<b>Galat.</b> Garis field merah dan kalimat di bawah kata sandi: siapa yang menolak. Email tetap terisi; fokus kembali ke kata sandi dan isinya terseleksi.', 'signin-expired-dark', '<b>Sesi habis.</b> Lede menjadi apa yang terjadi dan kapan; satu notis bahwa tidak ada yang direkam; "Sign in to resume" kembali ke kamera terakhir.');

two('Masuk — terang', '1280×800 · 1440×900', 'signin-1280-light', '<b>1280.</b> Kolom form tetap; stage mengecil.', 'signin-light', '<b>1440.</b> Halaman abu pucat; input di atas permukaan yang sedikit lebih terang.');

one('Pengaturan — Playback', 'gelap · 1440×900', 'settings-playback-dark', `
<h3>Bentuk</h3><p>Pengaturan adalah layar sendiri, bukan panel: navigasi lima bagian di kiri, satu bagian di kanan. "← Live" kembali ke kamera terakhir.</p>
<h3>Baris</h3><p>Label dan petunjuk di kolom 200 px, kontrol di kanan. Petunjuk membawa konsekuensi, bukan pengulangan label.</p>
<h3>Simpan</h3><p>Baris bawah selalu ada: "Everything saved", atau "Unsaved changes · 2 fields" dengan Discard dan Save. Preferensi browser (bagian ini) tersimpan langsung; konfigurasi server menunggu Save.</p>
<h3>Keyboard</h3><p>${kbd(',')} membuka Pengaturan dari mana saja; ${kbd('Esc')} kembali bila tidak ada perubahan; ${kbd('Ctrl')}+${kbd('S')} menyimpan.</p>`, { dark: true });

one('Info perangkat', 'Settings → This camera · terang', 'settings-device-light', `
<h3>Isi</h3><p>Semua yang dulu ada di panel Camera controls dan "What the device reports it can do" pindah ke sini: nama, id, koneksi, dan tabel kapabilitas dengan bukti tiap baris.</p>
<h3>Kenapa di sini</h3><p>Tabel ini menjawab "kenapa tombol Move tidak ada?". Pertanyaan sesekali, bukan setiap membuka kamera.</p>
<h3>Hapus</h3><p>"Remove from this app" teks merah dengan kalimat konsekuensi di atasnya; konfirmasi inline, bukan modal.</p>`);

two('Konfigurasi', 'Settings → Configuration · Playback 1280', 'settings-config-light', '<b>Ada perubahan.</b> Baris bawah menyebut jumlah field berubah. Field rahasia kosong dengan placeholder "stored, type to replace".', 'settings-playback-1280-light', '<b>Playback 1280×800.</b> Navigasi tetap 216 px; isi bergeser.',
  '<p>"Needs a restart" tinggal di petunjuk field itu sendiri. Setelah Save, bila ada field seperti itu, baris bawah menjadi "Saved. Restart the app to apply 1 field" dengan tombol Restart (desktop) atau kalimat untuk server.</p>');

const typeRows = [
  ['display', '30 px / 1.12 · 600 · −0.02em', 'Watch the camera from this desk', 'font:600 30px/1.12 "Plex Sans";letter-spacing:-0.02em'],
  ['heading', '21 px / 1.25 · 600 · −0.01em', 'No cameras on this account yet', 'font:600 21px/1.25 "Plex Sans";letter-spacing:-0.01em'],
  ['title', '17 px / 1.35 · 500', 'No picture from Garage', 'font:500 17px/1.35 "Plex Sans"'],
  ['ui', '15 px / 1.45 · 400–500', 'Gate camera', 'font:500 15px/1.45 "Plex Sans"'],
  ['body', '14 px / 1.5 · 400', 'The camera refused the stream. This is the server’s own answer.', 'font:400 14px/1.5 "Plex Sans"'],
  ['small', '13 px / 1.45 · 400', 'live, bottom lens, 0.4 s behind', 'font:400 13px/1.45 "Plex Sans"'],
  ['data', '11 px / 1.4 · Mono 400', '21:40:12 · RTSP 401 · 192.168.1.64:34567', 'font:400 11px/1.4 "Plex Mono"'],
].map(([n, s, t, st]) => `<div class="type-row"><span class="mono">${n}</span><span class="mono mute">${s}</span><span style='${st}'>${t}</span></div>`).join('');

page('Sistem — tipografi', `${head('Tipografi', 'IBM Plex Sans + IBM Plex Mono · SIL Open Font License 1.1')}
<div class="split"><div>${typeRows}</div><div class="notes">
<h3>Kenapa Plex</h3><p>Aplikasi ini jalan di WebView2, WKWebView, dan WebKitGTK. <span class="mono">system-ui</span> berarti Segoe, SF, atau Cantarell: tiga rupa berbeda. Plex dibundel lokal (5 woff2 subset latin, 101 KB total) sehingga sama di semua OS dan jalan tanpa internet.</p>
<p>Plex Sans tegak dan teknis tanpa terasa dingin; Plex Mono dari keluarga yang sama untuk jam, alamat, kode galat, sehingga berpasangan tanpa saling mirip.</p>
<h3>Lisensi</h3><p>SIL OFL 1.1, © IBM Corp. Boleh dibundel dan didistribusikan bersama aplikasi MIT; lisensinya ikut di <span class="mono">fonts/OFL.txt</span>.</p>
<h3>Aturan</h3><ul><li>Skala tetap dalam px, bukan fluid.</li><li>Tidak ada huruf kapital ber-tracking; satu-satunya kapital adalah wordmark.</li><li>Mono hanya untuk data mesin.</li></ul></div></div>`);

const roles = [
  ['bg', '#0f1112', '#dfe0e2', 'halaman, topbar'],
  ['bg-2', '#1b1d1f', '#f0f1f2', 'input, menu, permukaan terangkat'],
  ['bg-3', '#242629', '#d3d5d7', 'hover, item menu terpilih, skeleton'],
  ['line', '#37393c', '#b0b2b5', 'batas input, garis tabel'],
  ['line-soft', '#2d2f32', '#bdbec1', 'pemisah baris, topbar'],
  ['ink', '#f1f2f4', '#1c1e21', 'teks utama, isi tombol utama, seleksi'],
  ['ink-2', '#cacccf', '#424447', 'label kontrol, teks sekunder'],
  ['ink-3', '#a6a9ae', '#5e6166', 'jam, petunjuk, meta'],
  ['stage', '#030405', '#030405', 'latar video, kedua tema'],
  ['stage-ink', '#cfd1d3', '#d6d8da', 'judul di atas stage'],
  ['stage-ink-2', '#96989c', '#9c9fa2', 'teks sekunder di atas stage'],
  ['live', '#fc665f', '#b10d1a', 'sedang merekam; satu-satunya aksen'],
  ['ok', '#6cce8a', '#176b39', 'dot kamera online, "supported"'],
  ['warn', '#edb95d', '#875a04', 'beku, basi, gagal per tile'],
  ['bad', '#fb7172', '#b9232d', 'galat form, aksi destruktif'],
  ['focus', '#7dc3ff', '#0072cb', 'cincin fokus, hanya itu'],
];
page('Sistem — warna', `${head('Warna dan peran', 'OKLCH, netral nada 255 dengan kroma ≤ 0,008 · hex hasil konversi skrip')}
<div class="split"><div><table><thead><tr><th>Token</th><th>Gelap</th><th>Terang</th><th>Peran</th></tr></thead><tbody>
${roles.map((r) => `<tr><td class="num">--${r[0]}</td><td class="num">${sw(r[1])}${r[1]}</td><td class="num">${sw(r[2])}${r[2]}</td><td>${r[3]}</td></tr>`).join('')}
</tbody></table></div><div class="notes">
<h3>Strategi</h3><p>Restrained: grafit netral + warna keadaan. Tidak ada warna merek. Tombol utama memakai tinta, bukan aksen, sehingga warna di layar selalu berarti sesuatu.</p>
<h3>Satu arti per warna</h3><ul><li>Merah <span class="mono">live</span>: sedang merekam. Bukan seleksi, bukan mute.</li><li>Merah <span class="mono">bad</span>: galat yang perlu tindakan, selalu dengan ikon dan kalimat.</li><li>Hijau: online/didukung, hanya dot atau teks kecil.</li><li>Kuning: data tidak segar.</li><li>Biru: fokus keyboard saja.</li></ul>
<h3>Kenapa abu, bukan putih atau krem</h3><p>Halaman terang <span class="mono">#dfe0e2</span> supaya video tetap lebih terang dari chrome. Kroma 0,003: tidak hangat, tidak kebiruan.</p></div></div>`);

page('Sistem — ruang, bentuk, ikon, gerak', `${head('Ruang, radius, elevasi, ikon, gerak', 'src/tokens.css')}
<div class="grid3 notes">
<div><h3>Spasi</h3><p class="mono">4 · 6 · 8 · 12 · 16 · 20 · 24 · 32 · 40 · 56</p><p>Rapat di dalam kontrol (6–8), lega antar kelompok (16–24). Tepi halaman 20 px; video menempel ke tepi itu, bukan ke kolom 1440 px.</p>
<h3>Ukuran tetap</h3><dl class="kv"><dt>topbar</dt><dd>56 px</dd><dt>kontrol</dt><dd>baris 64 px, tombol 36 px</dd><dt>menu</dt><dd>baris 48 px</dd></dl></div>
<div><h3>Radius</h3><dl class="kv"><dt>4 px</dt><dd>kode, chip data</dd><dt>6 px</dt><dd>tombol, input, tile</dd><dt>10 px</dt><dd>stage, menu</dd><dt>pill</dt><dd>hanya switch dan status di tabel</dd></dl>
<h3>Elevasi</h3><p>Tiga lapis: halaman (datar), permukaan <span class="mono">bg-2</span> untuk input dan menu, bayangan hanya pada lapisan yang melayang (menu, popover). Tidak ada kartu berbayang; bayangan tidak pernah dipasangkan dengan border lebar.</p></div>
<div><h3>Ikon</h3><p>Satu set SVG buatan sendiri dalam sprite inline: kotak 20 px, stroke 1,5, ujung bulat; varian 16 px di menu. Tanpa font ikon, tanpa emoji, tanpa aset vendor. Ikon selalu dengan teks, kecuali tiga ikon navigasi dan layar penuh (nama aksesibel + tooltip).</p>
<h3>Gerak</h3><dl class="kv"><dt>120 ms</dt><dd>hover, tekan</dd><dt>180 ms</dt><dd>menu masuk: opacity + 4 px</dd><dt>kurva</dt><dd class="mono">cubic-bezier(.22,1,.36,1)</dd></dl><p>Tanpa animasi masuk halaman, tanpa pulsa di dot rekam. <span class="mono">prefers-reduced-motion</span>: durasi 0, garis memuat diam.</p></div>
</div>`);

const comps = [
  ['Tombol utama', 'isi tinta, 40 px, radius 6', 'Sign in, Save changes', 'aksi di layar Live'],
  ['Kontrol', 'ikon 20 + teks 14, tanpa bingkai, 36 px', 'Talk, Sound, Record, Snapshot, Move', 'navigasi'],
  ['Kontrol aktif', 'latar bg-2 + garis 1 px; Talk: isi tinta; Record: latar merah lembut', 'keadaan yang sedang berjalan', 'seleksi biasa'],
  ['Pemilih', 'teks 13 + ▾, tanpa bingkai', 'Lens, Quality, Layout', 'aksi'],
  ['Tombol ikon', '36×36, ikon 20', 'grid, pengaturan, tema, layar penuh', 'aksi yang mengubah kamera'],
  ['Tombol garis', 'garis 1 px, 36 px', 'Refresh, Try again', 'aksi utama'],
  ['Teks bergaris bawah', 'teks 14, underline offset 3 px', 'Open Settings, Discard, Retry now', 'aksi destruktif tanpa kalimat'],
  ['Dot', '8 px, selalu dengan teks atau nama aksesibel', 'status kamera, rekam', 'dekorasi'],
  ['Menu', 'bg-2, radius 10, baris 48 px, judul monospace kecil', 'kamera, lensa', 'form'],
  ['Banner', 'satu baris di bawah topbar, ikon kuning', 'kamera offline, jaringan hilang', 'sukses, info'],
  ['Keterangan', 'nama 14/500 + keadaan 13 + jam mono', 'di bawah setiap gambar', 'teks di atas video'],
];
page('Sistem — komponen', `${head('Satu kosakata', 'tiap komponen interaktif: default, hover, fokus, aktif, nonaktif, memuat')}
<table><thead><tr><th style="width:36mm">Komponen</th><th>Bentuk</th><th>Dipakai di</th><th>Tidak dipakai untuk</th></tr></thead><tbody>
${comps.map((r) => `<tr>${r.map((c) => `<td>${c}</td>`).join('')}</tr>`).join('')}
</tbody></table>`);

const keys = [
  ['1–9', 'pindah ke kamera ke-n'], ['G', 'Live ↔ Grid'], ['F', 'layar penuh'], ['M', 'suara on/off'],
  ['T (tahan)', 'bicara lewat interkom'], ['R', 'mulai/hentikan rekam'], ['S', 'snapshot'],
  ['L', 'putar lensa Both → Top → Bottom'], ['Q', 'HD ↔ SD'], ['V', 'buka/tutup pad gerak'],
  ['←↑→↓', 'gerakkan lensa (pad terbuka) / pindah tile (grid)'], ['Space', 'hentikan gerak'],
  [',', 'Pengaturan'], ['Esc', 'tutup menu → tutup pad → keluar layar penuh'], ['?', 'daftar pintasan'],
];
page('Interaksi — keyboard dan fokus', `${head('Keyboard dan urutan fokus', 'lengkap tanpa mouse')}
<div class="split half"><div class="notes"><h3>Pintasan global</h3><table><tbody>
${keys.map(([k, v]) => `<tr><td style="width:26mm">${kbd(k)}</td><td>${v}</td></tr>`).join('')}
</tbody></table><p class="cap">Pintasan huruf tidak aktif saat fokus di field teks. Daftar ini juga muncul lewat ${kbd('?')} sebagai lembar biasa, bukan modal.</p></div>
<div class="notes"><h3>Urutan fokus — Live</h3><ol style="padding-left:5mm;margin:0 0 3mm"><li>"Skip to controls" (tersembunyi sampai difokus)</li><li>Nama kamera ▾</li><li>Grid · Pengaturan · Tema</li><li>Stage (hanya bila pad gerak terbuka; panah bekerja di sini)</li><li>Talk · Sound · Record · Snapshot</li><li>Move · Lens · Quality · Layar penuh</li></ol>
<h3>Masuk</h3><ol style="padding-left:5mm;margin:0 0 3mm"><li>Tab Account / This network (panah kiri/kanan)</li><li>Email → Password → Sign in</li><li>Open Settings · Tema</li></ol>
<h3>Pengaturan</h3><ol style="padding-left:5mm;margin:0 0 3mm"><li>← Live</li><li>Navigasi bagian (panah atas/bawah)</li><li>Field bagian aktif, dari atas</li><li>Discard · Save</li></ol>
<h3>Setelah aksi</h3><p>Menu tertutup → fokus kembali ke pemicunya. Pindah kamera → fokus tetap di tempat. Galat masuk → fokus ke field yang salah.</p></div></div>`);

const contrastRows = readFileSync(join(here, 'contrast.md'), 'utf8').trim().split('\n').slice(2)
  .map((l) => l.split('|').slice(1, -1).map((s) => s.trim()));
const passCount = contrastRows.filter((r) => r[6] === 'pass').length;
const ctab = (theme) => `<table style="font-size:7.6pt"><thead><tr><th>Pasangan</th><th>Depan</th><th>Latar</th><th>Rasio</th><th>Batas</th></tr></thead><tbody>${contrastRows.filter((r) => r[0] === theme).map((r) => `<tr><td>${r[1]}</td><td class="num">${sw(r[2])}${r[2]}</td><td class="num">${sw(r[3])}${r[3]}</td><td class="num"><b>${r[4]}</b></td><td class="num">${r[5]}</td></tr>`).join('')}</tbody></table>`;
page('Aksesibilitas — kontras', `${head('Kontras, dihitung', `src/contrast.py · OKLCH → sRGB → luminans relatif WCAG 2.1 · ${passCount}/${contrastRows.length} lulus`)}
<div class="split half"><div><h3>Gelap</h3>${ctab('dark')}</div><div><h3>Terang</h3>${ctab('light')}</div></div>`);

page('Aksesibilitas — lainnya', `${head('Aksesibilitas', 'WCAG 2.1 AA')}
<div class="grid2 notes">
<div><h3>Batas kontras</h3><p>Teks 4,5:1 termasuk teks 11 px dan placeholder. Grafis (dot, cincin fokus) 3:1. Garis dekoratif <span class="mono">line</span> ≥ 1,6:1 dan <span class="mono">line-soft</span> ≥ 1,4:1: batas pilihan sendiri karena WCAG tidak mewajibkan untuk pemisah; batas input tidak bergantung pada garis saja, ada latar <span class="mono">bg-2</span>.</p>
<p>Teks di atas stage memakai token stage yang tidak berubah dengan tema. Pad gerak memakai latar gelap 90 % di atas video, jadi ikonnya terbaca di gambar putih sekalipun.</p></div>
<div><h3>Target</h3><p>Semua kontrol ≥ 36×36 px di desktop (tombol pad 40×40, baris menu 48 px). Dengan input sentuh (<span class="mono">pointer: coarse</span>) kontrol naik ke 44 px tanpa mengubah tata letak.</p>
<h3>Fokus terlihat</h3><p>Cincin 2 px <span class="mono">--focus</span> dengan jarak 2 px, hanya pada <span class="mono">:focus-visible</span>. 10,07:1 di gelap, 3,73:1 di terang, 4,16:1 di atas stage.</p></div>
<div><h3>Bukan hanya warna</h3><p>Setiap dot punya nama ("Online", "Offline", "Recording"). Beku/basi selalu ditulis dengan kata, bukan hanya kuning. Galat selalu ikon + kalimat.</p>
<h3>Live region</h3><p>Keterangan di bawah gambar <span class="mono">role="status"</span> yang hanya mengumumkan perubahan keadaan (live → beku → gagal), bukan detik jam.</p></div>
<div><h3>Gerak dikurangi</h3><p><span class="mono">prefers-reduced-motion: reduce</span> → durasi token 0, menu muncul tanpa geser, garis memuat diam. Meter level mikrofon tetap bergerak karena itu data, tanpa transisi.</p>
<h3>Bahasa</h3><p>Microcopy tetap Inggris seperti aplikasi sekarang; kalimat pendek dengan subjek jelas ("The camera refused the stream"), tanpa "Oops".</p></div></div>`);

const slop = [
  ['Hero di tengah dengan satu CTA', 'tidak ada', 'Masuk = form kiri + stage tempat gambar akan muncul. UI lama memakai pola ini.'],
  ['Semua kartu radius sama + bayangan', 'tidak ada', 'Tidak ada kartu. Bayangan hanya pada menu yang melayang.'],
  ['Ikon emoji', 'tidak ada', 'Sprite SVG 20 px buatan sendiri.'],
  ['Gradien ungu-biru', 'tidak ada', 'Tidak ada gradien sama sekali. Arah A sempat punya gradien biru di logo; dicabut.'],
  ['Teks 14 px abu di atas putih', 'tidak ada', 'Teks abu terkecil 4,69:1 (terang), dan halaman terang bukan putih.'],
  ['Dashboard berisi kartu statistik', 'tidak ada', '"4 cameras · 3 live · 1 offline" sebagai satu kalimat.'],
  ['Label Overview / Insights / Seamless', 'tidak ada', 'Label kata kerja atau nama benda: Talk, Record, This camera.'],
  ['Eyebrow kapital ber-tracking', 'tidak ada', 'Lima di UI lama. Satu-satunya kapital: wordmark.'],
  ['Border kiri berwarna sebagai aksen', 'tidak ada', 'Banner dan notis memakai latar, bukan garis samping.'],
  ['Glassmorphism / blur dekoratif', 'tidak ada', 'Pad gerak memakai latar gelap pekat; blur dicabut di putaran kritik.'],
  ['Radius ≥ 24 px', 'tidak ada', 'Maksimal 10 px.'],
  ['Latar krem/pasir', 'tidak ada', 'Netral kroma 0,003.'],
  ['Animasi masuk / pulsa "hidup"', 'tidak ada', 'Gerak hanya untuk keadaan. Dot rekam diam.'],
  ['Ilustrasi kosong / sketsa SVG', 'tidak ada', 'Empty state = kalimat + aksi.'],
  ['Teks di atas video', 'hanya layar penuh', 'Keterangan di bawah gambar. Di layar penuh satu baris mono kecil di tepi bawah.'],
  ['Angka yang dikarang', 'tidak ada', 'Angka dokumen ini diukur (DOM, contrast.py). Data di mock (jam, IP, id) contoh.'],
];
page('Anti-slop', `${head('Yang sengaja tidak dipakai', 'daftar ZHA-3941 + larangan skill impeccable')}
<table><thead><tr><th style="width:62mm">Penanda slop</th><th style="width:26mm">Status</th><th>Alasan / gantinya</th></tr></thead><tbody>
${slop.map((r) => `<tr><td>${r[0]}</td><td class="num">${r[1]}</td><td>${r[2]}</td></tr>`).join('')}
</tbody></table>`);

page('Kritik diri', `${head('Kritik diri sebelum dikirim', 'skor hallmark 1–5 · revisi bila di bawah 3')}
<div class="split"><div class="notes">
<table style="margin-bottom:4mm"><thead><tr><th>Sumbu</th><th>Skor</th><th>Catatan</th></tr></thead><tbody>
<tr><td>Philosophy</td><td class="num">5</td><td>Satu gagasan (jendela ke satu kamera) menjelaskan semua keputusan.</td></tr>
<tr><td>Hierarchy</td><td class="num">4</td><td>Gambar → nama → keterangan → kontrol. Ikon topbar agak jauh dari kontrol lain; diterima.</td></tr>
<tr><td>Execution</td><td class="num">4</td><td>Mock HTML/CSS nyata, bukan gambar. Belum diuji di WebKitGTK.</td></tr>
<tr><td>Specificity</td><td class="num">4</td><td>Kalimat menyebut kamera, port, jam. Wordmark sementara.</td></tr>
<tr><td>Restraint</td><td class="num">5</td><td>Satu aksen, tanpa kartu, tanpa gradien.</td></tr>
<tr><td>Variety</td><td class="num">4</td><td>Tidak mengikuti template rail + header + kartu.</td></tr></tbody></table>
<h3>Diperbaiki di putaran kritik</h3><ul><li>Pad gerak memakai blur → latar pekat (larangan glassmorphism).</li><li>Mock "Both lenses" memakai gambar 16:9 biasa → frame dua gambar bertumpuk seperti yang dikirim kamera.</li><li>Tile grid meng-crop → <span class="mono">contain</span>.</li><li>Lima pasangan kontras gagal di putaran pertama (ink-3, live, ok, bad di terang; garis) → token disesuaikan sampai semua lulus.</li><li>Arah A: gradien logo dan tombol biru dihapus supaya perbandingan adil.</li></ul></div>
<div class="notes"><h3>Risiko terbuka</h3><ul><li>Popover API dan <span class="mono">color-mix()</span> belum diverifikasi di WebKitGTK build Linux.</li><li>Gambar kamera di mock sintetis, dibuat untuk dokumen ini (bukan rekaman, bukan aset vendor). Gambar kamera asli bisa lebih gelap; stage hitam sudah memperhitungkannya.</li><li>PROVIEW adalah nama vendor; dipakai sebagai wordmark teks, bukan logo.</li></ul></div></div>`);

page('Implementasi', `${head('Realistis untuk Rust + HTML/CSS/JS biasa', 'tanpa framework, tanpa build step baru')}
<div class="grid2 notes">
<div><h3>Berubah</h3><ul><li><span class="mono">web/tokens.css</span> diganti token di dokumen ini (bg, ink, stage, live…).</li><li><span class="mono">web/style.css</span> ditulis ulang; lebih pendek karena panel dan segmented control hilang.</li><li><span class="mono">web/index.html</span>: rail dan panel Camera controls dihapus; Pengaturan jadi layar dengan navigasi bagian.</li><li><span class="mono">web/fonts/</span>: 5 woff2 + OFL.txt, disajikan Rocket dan dibundel Tauri.</li></ul></div>
<div><h3>JS tambahan</h3><ul><li>Menu kamera + lensa dengan fokus berputar: ±60 baris (lebih sedikit bila popover API tersedia).</li><li>Tahan-untuk-bicara (pointer + keyboard): ±30 baris di atas interkom yang ada.</li><li>Meter level mikrofon: <span class="mono">AnalyserNode</span>, ±25 baris.</li><li>Pintasan keyboard + lembar ${kbd('?')}: ±50 baris.</li><li>Baris simpan Pengaturan dengan jumlah field berubah: logika diff sudah ada.</li></ul></div>
<div><h3>Tetap</h3><p>Semua endpoint, keadaan stage, deteksi beku, crop lensa di browser, laporan kapabilitas, alur konfigurasi. Desain ini mengubah penyajian, bukan kontrak server.</p></div>
<div><h3>Urutan setelah disetujui</h3><ol style="padding-left:5mm;margin:0"><li>Token + font + layar Live satu kamera.</li><li>Menu kamera, grid, layar penuh.</li><li>Masuk dan semua keadaan galat.</li><li>Pengaturan + info perangkat.</li></ol></div></div>`);

page('Sumber', `${head('Sumber dan render ulang', 'docs/design/redesign-v1/')}
<div class="notes"><dl class="kv" style="grid-template-columns:56mm 1fr;font-size:9.5pt">
<dt>src/screen.html + screens.js</dt><dd>Semua layar arah B. <span class="mono">?screen=live&amp;theme=dark</span>; daftar layar di objek SCREENS.</dd>
<dt>src/app.css · src/tokens.css</dt><dd>Sistem visual; token di sini sumber angka di halaman Sistem.</dd>
<dt>src/dir-a.html · src/dir-c.html</dt><dd>Mock arah A dan C.</dd>
<dt>src/gen-doc.mjs · src/doc.css</dt><dd>Menghasilkan <span class="mono">src/doc.html</span>, dokumen ini.</dd>
<dt>src/render.sh</dt><dd>Server lokal + Chrome headless → <span class="mono">shots/*.jpg</span>.</dd>
<dt>src/build-pdf.sh</dt><dd>doc.html → <span class="mono">proview-redesign-v1.pdf</span>.</dd>
<dt>src/contrast.py</dt><dd>Menghitung semua pasangan kontras dari tokens.css, keluar non-nol bila ada yang gagal, menulis <span class="mono">contrast.md</span>.</dd>
<dt>src/fonts/</dt><dd>IBM Plex Sans/Mono woff2 subset latin + OFL.txt.</dd>
<dt>img/*.jpg</dt><dd>Frame kamera sintetis untuk mock, dan tangkapan UI lama (<span class="mono">old-*.jpg</span>).</dd>
</dl>
<p style="margin-top:6mm" class="mono">cd docs/design/redesign-v1 &amp;&amp; python3 src/contrast.py &amp;&amp; src/render.sh &amp;&amp; node src/gen-doc.mjs &amp;&amp; src/build-pdf.sh</p></div>`);

const html = `<!doctype html><html lang="id"><head><meta charset="utf-8"><title>PROVIEW redesign v1</title>
<link rel="stylesheet" href="tokens.css"><link rel="stylesheet" href="doc.css"></head><body>
${pages.join('\n')}
</body></html>
`;
writeFileSync(join(here, 'doc.html'), html);
console.log(`doc.html: ${pn} pages`);
