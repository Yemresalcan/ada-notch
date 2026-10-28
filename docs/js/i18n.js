/* Turkish / English localization for the Ada landing page. */
(() => {
  "use strict";

  const STORAGE_KEY = "ada-lang";
  const DEFAULT_VERSION = "0.1.1";

  const STRINGS = {
    tr: {
      "meta.title": "Ada — Windows için Dynamic Island",
      "meta.desc": "Ada, Windows 10 ve 11 için Rust ile yazılmış ücretsiz ve açık kaynak Dynamic Island tarzı bir çentik: müzik, şarkı sözleri, odak zamanlayıcısı, hava durumu ve dosya rafı.",

      "nav.features": "Özellikler",
      "nav.mascot": "Maskot",
      "nav.opensource": "Açık kaynak",
      "nav.faq": "SSS",
      "nav.download": "İndir",

      "hero.eyebrow": "Windows 10 & 11 için · Ücretsiz ve açık kaynak",
      "hero.title_html": "Ekranının tepesinde<br>küçük bir <em>ada</em>.",
      "hero.sub": "Müzik ve senkronize şarkı sözleri, odak zamanlayıcısı, hava durumu, dosya rafı ve bildirimler — hepsi ekranının tepesindeki zarif siyah kapsülde. Rust ile yazılmış minik ve yerleşik bir Windows uygulamasıdır; boştayken işlemciyi hiç yormaz.",
      "hero.cta": "Windows için indir",
      "hero.cta2": "GitHub'da incele",
      "hero.meta": "v{version} · ~700 KB · Kurulum yönetici izni istemez",
      "hero.hint": "Üstteki çentiğin üzerine gel",

      "feat.title": "Küçük bir çentik, bir sürü iş.",
      "feat.sub": "Her sayfa ayrı bir özellik; istemediğini Ayarlar'dan tek tıkla kapat.",
      "feat.media.t": "Müzik ve şarkı sözleri",
      "feat.media.d": "Herhangi bir uygulamada çalan parçayı kontrol et, senkron şarkı sözlerini takip et ve kapalı çentikte albüm kapağıyla canlı ses çubuklarını gör.",
      "feat.shelf.t": "Dosya rafı",
      "feat.shelf.d": "Dosya ve bağlantıları çentiğe sürükleyip geçici olarak bırak, lazım olduğunda dışarı geri sürükle.",
      "feat.focus.t": "Odak zamanlayıcısı",
      "feat.focus.d": "25/5/15 dakikalık Pomodoro döngüleriyle odaklan ve süreyi çentikte canlı geri sayımdan izle.",
      "feat.weather.t": "Hava durumu",
      "feat.weather.d": "IP tabanlı konumla bulunduğun yerin anlık hava durumunu ve saatlik tahminini kurulumsuz gör.",
      "feat.system.t": "Sistem",
      "feat.system.d": "CPU ve RAM kullanım halkalarını canlı akış grafiğiyle birlikte tek bakışta takip et.",
      "feat.notify.t": "Bildirimler",
      "feat.notify.d": "Windows bildirimlerini çalışmanı bölmeyen kısa birer “peek” önizlemesi olarak yakala.",
      "feat.todo.t": "Yapılacaklar ve takvim",
      "feat.todo.d": "Hızlı yapılacaklar listeni ve sade yerel takvimini ekranının tepesinde elinin altında tut.",
      "feat.settings.t": "Senin kuralların",
      "feat.settings.d": "Her sayfayı tek tek açıp kapat veya daha sade bir görünüm için minimal moda geç.",

      "mascot.title_html": "Boşta kalınca<br><em>Merhaba</em> der.",
      "mascot.sub": "15 saniye hareketsiz kaldığında el yazısıyla “Merhaba” yazar, gözlerini açar, uykuya dalıp el sallar; ekran başına döndüğünde uyanır.",
      "mascot.try": "Sayfada 5 saniye hiçbir şeye dokunma",

      "stat.size.v": "~700 KB",
      "stat.size.l": "tek exe, bağımlılık yok",
      "stat.cpu.v": "0 FPS",
      "stat.cpu.l": "boştayken",
      "stat.native.v": "Rust",
      "stat.native.l": "Direct2D + DirectComposition",
      "stat.pages.v": "10",
      "stat.pages.l": "sayfa, hepsi isteğe bağlı",

      "os.title": "Açık kaynak, sonuna kadar.",
      "os.sub": "Apache-2.0 lisanslıdır ve her sürüm herkese açık kaynak kodundan doğrudan GitHub Actions ile derlenir; katkılar ve fikirler her zaman memnuniyetle karşılanır.",
      "os.star": "GitHub'da yıldız ver",
      "os.contribute": "Katkıda bulun",

      "faq.title": "Sık sorulanlar",
      "faq.q1": "İndirirken tarayıcı veya Windows SmartScreen neden uyarı veriyor?",
      "faq.a1": "Ada yeni bir uygulamadır ve henüz kod imzalı değildir; bu nedenle tarayıcılar ve SmartScreen az indirilen dosyalar için standart bir itibar uyarısı gösterir, bu bir virüs tespiti değildir. Dosyayı SHA256SUMS.txt üzerinden SHA-256 ile doğrulayabilir ve “Ek bilgi → Yine de çalıştır” (More info → Run anyway) diyerek güvenle açabilirsin.",
      "faq.q2": "Verilerim nereye gidiyor?",
      "faq.a2": "Her şey bilgisayarında, %APPDATA%\\Ada klasöründe yerel olarak kalır; hiçbir telemetri gönderilmez. Ağ bağlantısı yalnızca ilgili sayfalar açıksa hava durumu (MET Norway, IP ile konum) ve şarkı sözleri (LRCLIB) için kullanılır.",
      "faq.q3": "Ada'yı nasıl kaldırırım?",
      "faq.a3": "Windows Ayarlar › Uygulamalar menüsünden istediğin zaman kaldırabilirsin; yerel verilerini silip silmemek isteğe bağlı olarak sorulur. Taşınabilir ada.exe sürümünde ise dosyayı silmen yeterlidir.",
      "faq.q4": "macOS veya Linux sürümü var mı?",
      "faq.a4": "Hayır, Ada yalnızca Windows 10 ve 11 içindir — çentik fikri Mac'ten doğdu, Ada ise bu deneyimi yerleşik olarak Windows'a getirir.",

      "dl.title": "Ada'yı indir",
      "dl.sub": "Ücretsiz ve açık kaynak — kurulum bir dakika bile sürmez.",
      "dl.setup": "Kurulum dosyası",
      "dl.setup_note": "Önerilen · Başlat menüsü ve otomatik başlatma",
      "dl.portable": "Taşınabilir",
      "dl.portable_note": "Kurulumsuz tek .exe",
      "dl.sha": "SHA-256 değerleri",
      "dl.req": "Windows 10 1903+ veya 11 · x64",
      "dl.all": "Tüm sürümler",

      "footer.license": "Apache-2.0 lisanslı",
    },

    en: {
      "meta.title": "Ada — Dynamic Island for Windows",
      "meta.desc": "Ada is a free, open-source Dynamic Island-style notch for Windows 10 and 11, written natively in Rust with media controls, synced lyrics, a focus timer, weather, and a file shelf.",

      "nav.features": "Features",
      "nav.mascot": "Mascot",
      "nav.opensource": "Open source",
      "nav.faq": "FAQ",
      "nav.download": "Download",

      "hero.eyebrow": "For Windows 10 & 11 · Free & open source",
      "hero.title_html": "A little <em>island</em><br>at the top of your screen.",
      "hero.sub": "Music and synced lyrics, a focus timer, weather, a file shelf, and notifications — all tucked into a sleek black pill at the top of your screen. Built natively in Rust as a tiny ~700 KB app that idles at 0 FPS.",
      "hero.cta": "Download for Windows",
      "hero.cta2": "View on GitHub",
      "hero.meta": "v{version} · ~700 KB · No admin rights required",
      "hero.hint": "Hover the notch above",

      "feat.title": "A tiny notch that does a lot.",
      "feat.sub": "Every page is an independent feature — turn off anything you don't need in Settings.",
      "feat.media.t": "Media & lyrics",
      "feat.media.d": "See what's playing from any app with playback controls, synced lyrics, and album art plus animated bars in the collapsed notch.",
      "feat.shelf.t": "File shelf",
      "feat.shelf.d": "Drag files and links onto the notch to stash them temporarily, then drag them right back out later.",
      "feat.focus.t": "Focus timer",
      "feat.focus.d": "Run 25/5/15-minute Pomodoro sessions with a live countdown right inside the notch.",
      "feat.weather.t": "Weather",
      "feat.weather.d": "Get local weather conditions and an hourly forecast automatically by IP location.",
      "feat.system.t": "System",
      "feat.system.d": "Keep an eye on CPU and RAM rings paired with a live usage graph.",
      "feat.notify.t": "Notifications",
      "feat.notify.d": "Catch incoming Windows notifications as brief, unobtrusive animated peeks.",
      "feat.todo.t": "Todo & calendar",
      "feat.todo.d": "Keep a quick todo list and a simple local calendar right at the top of your screen.",
      "feat.settings.t": "Your rules",
      "feat.settings.d": "Turn every individual page on or off, or switch to minimal mode for a slimmer idle strip.",

      "mascot.title_html": "Says <em>Merhaba</em><br>when you step away.",
      "mascot.sub": "After 15 seconds idle it writes “Merhaba” by hand, opens its eyes, falls asleep and waves — then wakes right up when you return.",
      "mascot.try": "Don't touch anything for 5 seconds",

      "stat.size.v": "~700 KB",
      "stat.size.l": "single exe, no dependencies",
      "stat.cpu.v": "0 FPS",
      "stat.cpu.l": "when idle",
      "stat.native.v": "Rust",
      "stat.native.l": "Direct2D + DirectComposition",
      "stat.pages.v": "10",
      "stat.pages.l": "pages, all optional",

      "os.title": "Open source, through and through.",
      "os.sub": "Licensed under Apache-2.0 and built directly from public source by GitHub Actions — contributions, bug reports, and ideas are always welcome.",
      "os.star": "Star on GitHub",
      "os.contribute": "Contribute",

      "faq.title": "Frequently asked questions",
      "faq.q1": "Why does my browser or Windows SmartScreen show a warning?",
      "faq.a1": "Ada is new and not yet code-signed, so browsers and SmartScreen show a standard reputation warning — not a virus detection. Verify the download with SHA-256 against SHA256SUMS.txt, then choose “More info → Run anyway”.",
      "faq.q2": "Where does my data go?",
      "faq.a2": "Everything stays local in %APPDATA%\\Ada with zero telemetry. Only weather (MET Norway, location by IP) and synced lyrics (LRCLIB) are fetched when enabled.",
      "faq.q3": "How do I uninstall Ada?",
      "faq.a3": "Uninstall anytime from Settings › Apps, with an optional prompt to remove your local data. For the portable ada.exe, simply delete the file.",
      "faq.q4": "Is there a macOS or Linux version?",
      "faq.a4": "Windows only — the notch idea comes from Mac, and Ada brings it natively to Windows 10 and 11.",

      "dl.title": "Download Ada",
      "dl.sub": "Free and open source — setup takes under a minute.",
      "dl.setup": "Installer",
      "dl.setup_note": "Recommended · Start menu & autostart",
      "dl.portable": "Portable",
      "dl.portable_note": "Single .exe, no install",
      "dl.sha": "SHA-256 checksums",
      "dl.req": "Windows 10 1903+ or 11 · x64",
      "dl.all": "All releases",

      "footer.license": "Apache-2.0 licensed",
    },
  };

  if (!window.ADA_VERSION) {
    window.ADA_VERSION = DEFAULT_VERSION;
  }

  const readSavedLang = () => {
    try {
      const saved = localStorage[STORAGE_KEY];
      if (saved === "tr" || saved === "en") return saved;
    } catch (_) {
      // Storage unavailable.
    }
    return null;
  };

  const writeSavedLang = (lang) => {
    try {
      localStorage[STORAGE_KEY] = lang;
    } catch (_) {
      // Storage unavailable.
    }
  };

  const detectLang = () => {
    const saved = readSavedLang();
    if (saved) return saved;
    const nav = (navigator.language || "").toLowerCase();
    return nav.startsWith("tr") ? "tr" : "en";
  };

  const escapeHtml = (s) =>
    s.replace(/[&<>"']/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[ch]);

  let activeLang = detectLang();

  function apply(lang) {
    const next = typeof lang === "string" && lang.toLowerCase().startsWith("tr")
      ? "tr"
      : lang === "en"
        ? "en"
        : activeLang;
    activeLang = next;
    document.documentElement.lang = next;

    const dict = STRINGS[next] || STRINGS.en;
    const version = String(window.ADA_VERSION || DEFAULT_VERSION);
    const safeVersion = escapeHtml(version);

    const resolve = (key) => {
      const raw = dict[key] ?? STRINGS.en[key];
      return raw === undefined ? undefined : raw.replace(/\{version\}/g, () => version);
    };

    const title = resolve("meta.title");
    if (title !== undefined) document.title = title;

    const desc = resolve("meta.desc");
    const metaDesc = document.querySelector('meta[name="description"]');
    if (metaDesc && desc !== undefined) metaDesc.setAttribute("content", desc);

    document.querySelectorAll("[data-i18n]").forEach((el) => {
      const val = resolve(el.getAttribute("data-i18n"));
      if (val !== undefined) el.textContent = val;
    });

    document.querySelectorAll("[data-i18n-html]").forEach((el) => {
      const key = el.getAttribute("data-i18n-html");
      const raw = dict[key] ?? STRINGS.en[key];
      if (raw === undefined) return;
      if (/<(br|em)\b/i.test(raw)) {
        el.innerHTML = raw.replace(/\{version\}/g, () => safeVersion);
      } else {
        el.textContent = raw.replace(/\{version\}/g, () => version);
      }
    });

    document.querySelectorAll("[data-i18n-aria]").forEach((el) => {
      const val = resolve(el.getAttribute("data-i18n-aria"));
      if (val !== undefined) el.setAttribute("aria-label", val);
    });

    const toggle = document.getElementById("lang-toggle");
    if (toggle) {
      toggle.textContent = next === "tr" ? "EN" : "TR";
    }
  }

  function current() {
    return activeLang;
  }

  window.adaI18n = { apply, current };

  const init = () => {
    const toggle = document.getElementById("lang-toggle");
    if (toggle) {
      toggle.addEventListener("click", () => {
        const next = activeLang === "tr" ? "en" : "tr";
        writeSavedLang(next);
        apply(next);
      });
    }
    apply(activeLang);
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init, { once: true });
  } else {
    init();
  }
})();
