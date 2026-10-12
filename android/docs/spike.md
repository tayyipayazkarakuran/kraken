# Android: Faz 0 incelemesi ve yol haritası

## Neden WebView?

Windows sürümü 32-bit IL2CPP istemcisine DLL enjekte ediyor, oyunu `il2cpp_*` API'si ile okuyor ve D3D11 üzerine çiziyor. Bunların hiçbiri telefonda yok. Tarayıcı sürümünde ise aynı C# kodu IL2CPP → C++ → **wasm32** olarak derlenmiş durumda:

- Oyunun bütün nesneleri tek bir `WebAssembly.Memory` içinde duruyor. `HEAPU8` üzerinden okunup yazılabiliyorlar.
- Metotlar wasm fonksiyon tablosundaki indeksler ile çağrılıyor (`table.get(i)(...)`).
- wasm32'de işaretçiler 32 bit. Windows ajanı da i686 (32-bit) olduğu için IL2CPP nesne düzenleri büyük ölçüde aynı.
- Unity WebGL tek iş parçacıklı çalışıyor. Oyuna yapılan çağrılar Unity'nin kare döngüsü içinden yapılmalı. Bu, Windows'taki `pump.rs` modelinin karşılığı.

## Faz 0: telefonda (veya masaüstü Chrome'da) yapılacak kontroller

Bu adımlar bulut ortamından yapılamadı: `seafight.com` ve `bot.ryik.dev` ağ politikası nedeniyle engelli (proxy 403).

1. **Build dosyaları.** DevTools → Network'te `*.loader.js`, `*.framework.js`, `*.wasm` (veya `.wasm.br`/`.unityweb`) ve `*.data` dosyalarının adlarını ve boyutlarını not edin.
2. **`__kraken.probe()`.** APK'da **K → Modülü incele**, masaüstünde konsoldan çalıştırılır. Çıktı şunları gösterir:
   - `exports`: export sayısı. `il2cpp`: export edilen `il2cpp_*` fonksiyonları.
   - `nameSection`: wasm'da fonksiyon isimleri var mı?
   - `memoryMiB` ve `table`: bellek ve fonksiyon tablosu boyutu.
3. **Metadata.** `.data` paketinden `global-metadata.dat` çıkarılıp `.wasm` ile birlikte Il2CppDumper (WebGL desteği var) ile incelenmeli. Windows istemcisindeki sınıflar (`Seafight.Network.NetworkManager`, `AttackSystem`, `GameActorModel`, `MapChangeRequestMessage` …) burada da aynı isimlerle bulunuyor mu?
4. **Karar.**
   - `il2cpp_*` fonksiyonları export ediliyorsa veya isim bölümünden bulunabiliyorsa: `il2cpp/registry.rs` ve `meta.rs` içindeki dinamik çözümleme mantığı wasm backend'i ile aynen kullanılır.
   - Değilse: modül yüklenmeden önce bayt kodu yamalanıp gereken fonksiyonlar export edilir (`state.modules[].bytes` bu iş için saklanıyor) ya da metadata tabanlı bir çözümleyici yazılır. Sabit indeks kullanılmamalı, çünkü her oyun güncellemesinde indeksler değişir.

## Başka bir botun incelemesinden öğrenilenler (yalnızca yöntem)

- Giriş, seafight.com'un kendisindeki Bigpoint formunda yapılıyor (`bgcdw_login_form`). Oyun sayfasına `internalStart` → `internalMapUnity` ile geçiliyor.
- Oyunun kodu `fetch` ile `*.wasm.gz` olarak iniyor. O bot bu indirmeyi bekletip kendi wasm-bindgen modülünü yüklüyor, ardından oyunun devam etmesine izin veriyor. Bu, wasm'ın örneklenmeden önce yamalandığına işaret ediyor; il2cpp API'si büyük ihtimalle dışarıya açık değil. `kraken-boot.js` bu indirmeleri `state.downloads` içinde kaydediyor; **İncele** çıktısında görünüyorlar.

## Yol haritası

- **Faz 1 (bitti):** `kraken-agent`'ın Windows'a bağlı kısımları `cfg(windows)` arkasında. Motor wasm'da derleniyor ve testleri Linux'ta da koşuyor. `kraken-web`, `kraken-boot.js`, Android kabuğu ve `cargo xtask web|android` hazır.
- **Faz 2:** wasm il2cpp backend'i, yani bellek okuma/yazma ve tablo üzerinden çağrı (Faz 0 kararına göre). Ardından `game/read.rs` ve `game/act.rs` mantığının bu backend'e taşınması: `World` taraması ve `Action` → oyun mesajı. Motor tick'i Unity'nin kare döngüsüne bağlanacak.
- **Faz 3:** Oyun içi panel. `kraken-app` içindeki egui arayüzü dokunmatik için ölçeklenip oyunun üstündeki şeffaf bir canvas'a çizilecek. Ayarlar `localStorage`/IndexedDB'de tutulacak.
- **Faz 4:** İmzalı release APK ve sürüm dağıtımı.
