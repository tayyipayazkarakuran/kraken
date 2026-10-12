# Kraken Android

Oyunun tarayıcı (Unity WebGL) sürümünü telefonda açan ve Kraken'ı aynı sayfanın içinde çalıştıran APK. Windows'taki DLL enjeksiyonunun yerini burada bir WebView ve sayfa başında çalışan bir betik alır. Kararları yine aynı motor (`crates/kraken-agent/src/engine`) verir.

> **Telefonda birincil yol artık web sitesi:** [../web/README.md](../web/README.md). APK isteğe bağlı ve şimdilik geliştirilmiyor; `kraken-web` ve `kraken-boot.js` ise her iki yolun ortak çekirdeği.

> **Durum: Faz 1 (iskelet).** APK oyunu açıyor, Kraken'ı sayfaya yüklüyor ve Unity'nin wasm modülünü yakalıyor. Motor başlatılıp durdurulabiliyor. Oyun durumunu okuma ve eylemleri oyuna iletme (Faz 2) henüz yok, bu yüzden çalışan motor şimdilik boşta bekliyor. Yol haritası için [docs/spike.md](docs/spike.md) dosyasına bakın.

## Nasıl çalışır

```
APK (MainActivity, WebView)
 ├─ kraken-boot.js  her çerçeveye sayfa başında enjekte edilir (addDocumentStartJavaScript)
 │    WebAssembly.instantiate / instantiateStreaming sarılır → Unity'nin memory'si ve fonksiyon tablosu yakalanır
 │    → kraken-web yüklenir, motor oyun çerçevesinde çalışır
 ├─ /__kraken/*     kraken-web dosyaları APK assets'inden, oyunun kendi origin'i altında sunulur (CSP engeli olmaz)
 └─ "K" düğmesi     Başlat / Duraklat / Durdur / Modülü incele / Oyun adresi / Yeniden yükle
```

| Yol | İçerik |
|---|---|
| `kraken-web/` | Rust crate (wasm32): motoru JS'e açar (`Kraken`, `configure`, `control`, `tick`) |
| `kraken-web/js/kraken-boot.js` | Sayfa başı betiği: wasm yakalama, kraken-web yükleme, çerçeveler arası komut aktarımı |
| `app/` | Android uygulaması (Kotlin, tek `MainActivity`) |
| `docs/spike.md` | Faz 0 inceleme adımları ve yol haritası |

## Derleme

Gerekenler: Rust (`rust-toolchain.toml`, `wasm32-unknown-unknown` hedefini kurar), Cargo.lock'taki sürümle aynı `wasm-bindgen-cli`, Android SDK (API 35) ve JDK 17.

```bash
cargo install wasm-bindgen-cli --version 0.2.129
cargo xtask web       # kraken-web → android/app/src/main/assets/kraken/
cargo xtask android   # web + ./gradlew assembleDebug
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
```

`local.properties` dosyasında `sdk.dir` tanımlı olmalı ya da `ANDROID_HOME` ayarlanmış olmalı. Android Studio ile `android/` klasörü doğrudan açılabilir.

## Kullanım

1. Uygulamayı açın. İlk açılışta `https://www.seafight.com/` yüklenir. Oyunun web adresi farklıysa **K → Oyun adresini değiştir** ile değiştirin.
2. Giriş yapıp oyunu başlatın.
3. **K → Modülü incele** seçeneği oyunun wasm modülü hakkında bir özet yazar. Başlat ve Durdur motoru kontrol eder.

Konsolu görmek için telefonu USB ile bağlayıp masaüstü Chrome'da `chrome://inspect` sayfasını açın. Debug APK'da WebView hata ayıklaması açıktır.

## Masaüstünde deneme

`kraken-boot.js`, Android olmadan masaüstü Chrome'da da çalışır:

1. Bir klasörde `cargo xtask web` çıktısını CORS izinli bir yerel sunucu ile yayınlayın.
2. Oyun yüklenmeden önce DevTools → Sources → Snippets'te `window.__KRAKEN_BASE__ = "http://127.0.0.1:8765/";` ayarlayıp `kraken-boot.js` içeriğini çalıştırın. Alternatif olarak document-start ile çalışan bir userscript kullanın.
3. Konsolda `__kraken.probe()`, `__kraken.start()` ve `__kraken.stop()` komutlarını kullanın.
