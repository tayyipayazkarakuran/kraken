# Kraken Web

Kraken'ı iPhone/iPad, Android ve bilgisayarda oyunun tarayıcı sürümünde çalıştıran Cloudflare sitesi.

> **Durum:** Site, userscript, panel ve sahip proxy'si çalışıyor. Kraken oyunun wasm modülünü yakalıyor ve motoru çalıştırıyor; oyunu okuma ve oynatma bir sonraki adım (bkz. [../android/docs/spike.md](../android/docs/spike.md)).

## Nasıl çalışır

| Yol | Kim için | Ne yapar |
|---|---|---|
| `/` | herkes | Platforma göre kurulum adımları, "Kraken'ı kur" ve "Oyunu aç" düğmeleri |
| `/kraken.user.js` | herkes | Userscript. Oyun resmi sitesinde açılır, giriş orada yapılır, şifre bu siteden geçmez |
| `/__kraken/*` | herkes | kraken-web (wasm + glue) ve `kraken-boot.js` |
| `/play/` | yalnızca site sahibi | Eklentisiz kullanım için proxy: oyunu bu siteden sunar ve `kraken-boot.js`'i ekler |

**Userscript yolu:** Betik, `kraken-boot.js`'i sayfa başında oyunun içine ekler. Bu betik Unity'nin wasm'ını yakalar ve oyun modülü görünce kraken-web'i ister. Userscript kraken-web'i `GM.xmlHttpRequest` ile bu siteden çekip sayfaya verir, böylece oyunun CORS kuralları devreye girmez. Bir sınır var: sayfa satır içi (inline) betikleri yasaklayan katı bir CSP kullanırsa ön betik eklenemez. Bu durumda konsola `[kraken] the page blocked the bootstrap (CSP)` uyarısı düşer.

**Proxy yolu (`/play/`):** Worker, oyunun sayfalarını `/play/<host>/…` altında sunar. `<head>`'in başına `proxy-shim.js` (oyunun fetch/XHR/WebSocket adreslerini proxy içinde tutar) ve `kraken-boot.js` eklenir. Çerezler `/play/<host>/` yoluna taşınır, CSP başlıkları kaldırılır, WebSocket'ler aktarılır. Yalnızca `PROXY_HOSTS` listesindeki adreslere gidilir. Hiçbir istek, çerez veya gövde loglanmaz. Erişim **Cloudflare Access** ile korunur: Worker, Access'in imzaladığı JWT'yi kendisi de doğrular. `ACCESS_TEAM_DOMAIN` ya da `ACCESS_AUD` boş olduğu sürece `/play/` herkese 403 döner.

Giriş formu oyunun kendi sitesinde olduğu için proxy modunda giriş de proxy'den geçer. Bu yüzden proxy yalnızca site sahibine açıktır. Herkese açık hale getirilmemeli.

## Kurulum

Gerekenler: Node 20+, Rust ve `wasm-bindgen-cli` (bkz. kök README) ve bir Cloudflare hesabı.

```bash
cargo xtask web          # kraken-web → web/public/__kraken/ (+ userscript)
cd web
npm install
npm test                 # proxy ve Access birim testleri
npx wrangler dev         # http://127.0.0.1:8787
npx wrangler login
npx wrangler deploy      # https://kraken.<hesap>.workers.dev
```

Özel alan adı için Cloudflare panelinde Workers → kraken → Settings → Domains & Routes kullanılır.

### Proxy modunu açma (isteğe bağlı, yalnızca sahip için)

1. Cloudflare Zero Trust → Access → Applications → **Self-hosted** uygulaması oluşturun. Alan adı sitenizin adresi, yol `play` olsun. Politika olarak yalnızca kendi e-postanıza izin verin.
2. Uygulamanın **Application Audience (AUD) Tag** değerini ve takım adresini (`https://<takım>.cloudflareaccess.com`) `wrangler.toml` içindeki `ACCESS_AUD` ve `ACCESS_TEAM_DOMAIN` alanlarına yazıp yeniden deploy edin.
3. `https://<site>/play` adresini açın. Access girişinden sonra oyun bu siteden açılır.

## Dosyalar

| Dosya | İçerik |
|---|---|
| `src/worker.js` | Yönlendirme, userscript, proxy |
| `src/proxy.js` | URL, çerez ve yönlendirme yeniden yazımı (saf fonksiyonlar) |
| `src/access.js` | Cloudflare Access JWT doğrulaması |
| `public/` | Karşılama sayfası, PWA manifest, `proxy-shim.js` |
| `userscript.template.js` | `cargo xtask web` bundan `public/__kraken/userscript.js` üretir |
| `test/` | `node --test` birim testleri |
