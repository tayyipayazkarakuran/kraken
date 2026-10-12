# Kraken

[English](README.md)

Kraken, Windows'ta çalışan Seafight oyun istemcisi (Unity / IL2CPP) için bir otomasyon yardımcısıdır. Oyunun içinde küçük bir ajan çalışır, oyunun durumunu okur ve işlemleri oyunun kendi sistemleri üzerinden yapar. Kontrol paneli oyun penceresinin içine çizilir; istenirse ayrı bir pencerede de açılabilir.

> **Uyarı.** Bu, Bigpoint ya da Seafight ile bağlantısı olmayan bağımsız bir hobi projesidir. Oyunu otomatikleştirmek kullanım şartlarına aykırı olabilir ve hesap kısıtlamasına yol açabilir. Kullanım sorumluluğu size aittir.

## Özellikler

| Alan | Ne yapar |
|---|---|
| NPC ve canavarlar | Listelerinizdeki hedefleri önceliğe göre seçer, yaklaşır, seçtiğiniz cephane ya da zıpkınla ateş eder |
| Toplayıcı | Seçtiğiniz kutu türlerini toplar; savaş sırasında yakındaki kutular için kısa sapmalar da yapar |
| Tamir | Eşiğin altına düşünce tamir eder; bu sırada ateş eden geminin tersindeki köşeye çekilir |
| Ölüm ve bağlantı | Standart seçenekle yeniden doğar; kısa kopmalarda yeniden bağlanır, uzun kesintilerde bekler |
| Bonus haritalar | Sıradaki bonus haritaları tek tek oynar, ateşini dalgalardaki gemilere dağıtır |
| El Dorado | Usta becerisi dolunca en yakın köşeye çekilir, atlar ve sandıkları toplar |
| Workmap | Listedeki her haritada belirlenen süre kadar çalışır, sonra sıradakine geçer; mümkünse ışınlanır, değilse kenardan kenara gider |
| War yardımcısı | PvP'de dümen oyuncudadır; Kraken hedef seçer, ateş eder ve eşya kurallarını uygular |
| Eşyalar | Aksiyon eşyalarını sahneye, koşula ve aralığa göre kullanma kuralları |
| Oyun içi arayüz | HUD, menzil halkası ve hedef işaretleri; F9 ile tam panel |
| Çoklu istemci | Her oyun istemcisi için ayrı oturum; ayarlar hesap başına saklanır |

## İndirme

[Releases](../../releases) sayfasından `Kraken-<sürüm>-win64.zip` dosyasını indirin, açın ve `kraken.exe`'yi çalıştırın. Her şey bu tek dosyanın içindedir: ajan ve injector gömülüdür, ilk açılışta `%LOCALAPPDATA%\Kraken\bin` klasörüne çıkarılır.

1. `kraken.exe`'yi başlatın. İsterseniz Seafight giriş ekranını sizin yerinize açar (Ayarlar → Bağlantı).
2. Giriş yapıp **Oyna**'ya basın. Kraken açılan oyuna kendiliğinden bağlanır.
3. Oyunda **F9** ile paneli açın, hedeflerinizi ayarlayın ve **Başlat**'a basın.

Ayarlar, kayıtlar ve oyun verisi önbelleği `%LOCALAPPDATA%\Kraken` klasöründe tutulur.

## Kaynaktan derleme

Windows ve Rust araç zinciri gerekir. `rust-toolchain.toml` hem `x86_64` hem `i686` MSVC hedeflerini kurar.

```bash
cargo xtask dist
```

Bu komut 32 bit ajanı ve injector'ı derler, 64 bit uygulamanın içine gömer ve sonucu `dist/Kraken/kraken.exe` olarak yazar.

```bash
cargo xtask check
```

Bu komut biçim denetimini, clippy'yi (`-D warnings`) ve bütün testleri çalıştırır.

## Proje yapısı

| Yol | İçerik |
|---|---|
| `crates/kraken-app` | Kontrol paneli (egui), oturum yöneticisi, oyun verisi (loca paketleri), oyun içi arayüz kareleri |
| `crates/kraken-agent` | Oyunun içindeki DLL: IL2CPP bağlamaları, mesaj kancaları, D3D11 katmanı ve karar motoru (`src/engine`) |
| `crates/kraken-injector` | Küçük bir 32 bit injector |
| `crates/kraken-proto` | Uygulama ↔ ajan protokolü: ayarlar, telemetri ve isimli boru (named pipe) çerçeveleme |
| `android` | Android uygulaması (oyunun tarayıcı sürümünü saran WebView) ve wasm'a derlenen motor `kraken-web`. Geliştirme aşamasında, bkz. [android/README.md](android/README.md) |
| `xtask` | `dist`, `check`, `reload` (geliştirme sırasında ajanı yeniden yükler), `web` ve `android` |

Motor, oyun çağrısı içermeyen saf Rust kodudur. Bir `World` anlık görüntüsü alır ve `Action` listesi döndürür. Bu yüzden bütün davranışı (hedef seçimi, tamir kaçışı, bonus haritalar, Workmap rotası ve diğerleri) birim testleriyle sınanır.

## Lisans

Tüm hakları saklıdır. Kaynak kod okunup incelensin diye paylaşılmıştır.
