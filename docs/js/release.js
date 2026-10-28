/* Fetches the latest GitHub release assets, version, and star count for Ada. */
(() => {
  "use strict";

  const REPO_URL = "https://api.github.com/repos/Yemresalcan/ada-notch";
  const RELEASE_URL = `${REPO_URL}/releases/latest`;
  const RELEASE_CACHE_KEY = "ada-release-latest-v1";
  const REPO_CACHE_KEY = "ada-repo-meta-v1";
  const CACHE_TTL_MS = 10 * 60 * 1000;

  const readCache = (key) => {
    try {
      const raw = sessionStorage.getItem(key);
      if (!raw) return null;
      const entry = JSON.parse(raw);
      if (!entry || typeof entry.ts !== "number" || Date.now() - entry.ts > CACHE_TTL_MS) {
        return null;
      }
      return entry.data ?? null;
    } catch (_) {
      return null;
    }
  };

  const writeCache = (key, data) => {
    try {
      sessionStorage.setItem(key, JSON.stringify({ ts: Date.now(), data }));
    } catch (_) {
      // Ignore quota / storage errors.
    }
  };

  const fetchCachedJson = async (url, cacheKey) => {
    const cached = readCache(cacheKey);
    if (cached) return cached;
    const res = await fetch(url, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json();
    writeCache(cacheKey, data);
    return data;
  };

  const formatBytes = (bytes) => {
    if (typeof bytes !== "number" || !Number.isFinite(bytes) || bytes <= 0) return null;
    if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
    return `${bytes} B`;
  };

  const setHref = (el, url) => {
    if (el && typeof url === "string" && url.startsWith("https://")) {
      el.setAttribute("href", url);
    }
  };

  const applyRelease = (release) => {
    if (!release || typeof release !== "object") return;

    const rawTag = typeof release.tag_name === "string" ? release.tag_name.trim() : "";
    const version = rawTag.replace(/^v/i, "");
    if (version) {
      window.ADA_VERSION = version;
      window.adaI18n?.apply(window.adaI18n.current());
      document.querySelectorAll("[data-version]").forEach((el) => {
        el.textContent = version;
      });
    }

    const assets = Array.isArray(release.assets) ? release.assets : [];
    const setup = assets.find((a) => a && typeof a.name === "string" && /^Ada-Setup-.*\.exe$/.test(a.name));
    const portable = assets.find((a) => a && a.name === "ada.exe");
    const sha = assets.find((a) => a && a.name === "SHA256SUMS.txt");

    if (setup?.browser_download_url) {
      setHref(document.getElementById("dl-setup"), setup.browser_download_url);
      document.querySelectorAll('[data-download="setup"]').forEach((el) => {
        setHref(el, setup.browser_download_url);
      });
      const sizeText = formatBytes(setup.size);
      if (sizeText) {
        document.querySelectorAll('[data-size="setup"]').forEach((el) => {
          el.textContent = sizeText;
        });
      }
    }

    if (portable?.browser_download_url) {
      setHref(document.getElementById("dl-portable"), portable.browser_download_url);
      const sizeText = formatBytes(portable.size);
      if (sizeText) {
        document.querySelectorAll('[data-size="portable"]').forEach((el) => {
          el.textContent = sizeText;
        });
      }
    }

    if (sha?.browser_download_url) {
      setHref(document.getElementById("dl-sha"), sha.browser_download_url);
    }
  };

  const applyRepo = (repo) => {
    if (!repo || typeof repo.stargazers_count !== "number") return;
    const count = repo.stargazers_count;
    if (count <= 0) return;
    const text = count >= 1000 ? `${(count / 1000).toFixed(1).replace(/\.0$/, "")}k` : String(count);
    document.querySelectorAll("[data-stars]").forEach((el) => {
      el.textContent = text;
    });
  };

  const run = async () => {
    const results = await Promise.allSettled([
      fetchCachedJson(RELEASE_URL, RELEASE_CACHE_KEY).then(applyRelease),
      fetchCachedJson(REPO_URL, REPO_CACHE_KEY).then(applyRepo),
    ]);
    const failed = results.find((r) => r.status === "rejected");
    if (failed) {
      console.debug("Ada release metadata fallback:", failed.reason);
    }
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", run, { once: true });
  } else {
    run();
  }
})();
