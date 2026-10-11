# speedwheel5-rs

Driver de vibración (force feedback) para el volante **Genius Speed Wheel 5 Pro**
(USB `1A5C:3112` / `1A5C:3012`), escrito en Rust, para **Windows y Linux**.

Reemplaza al driver original de JessTech (2008), que en Windows de 64 bits podía
cerrar los juegos, no combinaba los efectos y solo hacía funcionar 7 de los 12
efectos que anunciaba.

| Plataforma | Cómo funciona | Estado |
|---|---|---|
| Windows 10/11, juegos de 32 y 64 bits | Effect driver de DirectInput (DLL COM) | Probado con el volante |
| Linux (systemd + udev) | Servicio: uinput + hidraw | Compila y tiene tests; falta probarlo con el volante |

El volante tiene **dos motores de vibración**, no un motor de torsión: vibra, no
gira solo. Detalles del protocolo en [`docs/protocolo.md`](docs/protocolo.md).

## Instalar

Bajá el paquete de tu sistema desde **Releases**.

### Windows

1. Descomprimí `SpeedWheel5-driver-<versión>.zip` en una carpeta.
2. Doble clic en `1 - Instalar.cmd` y aceptá el aviso de administrador.
3. Doble clic en `2 - Probar.cmd`: tienen que sonar 7 vibraciones.

Si aparece "Windows protegió su PC", hacé clic en "Más información" y después en
"Ejecutar de todas formas": los binarios todavía no tienen firma digital. Para
desinstalar, usá `Desinstalar.cmd` o "Agregar o quitar programas": se restaura la
configuración anterior.

### Linux

```sh
tar xzf sw5-linux-<versión>-x86_64.tar.gz && cd sw5-linux-<versión>
sudo ./sw5-linux install      # binario, regla de udev, módulo uinput y servicio systemd
sw5-linux test                # 5 vibraciones a través del volante virtual
sudo sw5-linux uninstall
```

El binario es estático (musl), así que funciona en cualquier distribución con
systemd. En Steam Deck: `--prefix /home/deck/.local` (todavía sin probar).

## Cómo funciona

```text
Windows:  juego ── DirectInput 8 ──▶ sw5_effectdrv.dll (x86/x64) ─┐
Linux:    juego ── evdev FF (SDL, Steam, Proton) ──▶ sw5-linux ───┤
                                                                   ▼
                   sw5-core::Engine: suma todos los efectos cada 4 ms
                                                                   ▼
                      output report [00 00 00 A B] ──▶ volante (HID)
```

| Crate | Plataforma | Qué hace |
|---|---|---|
| `sw5-core` | todas (`no_std`) | Modelo de efectos con envolventes, fase, delay e iteraciones; mezcla en los dos motores con saturación; pausa; ganancia; codificación del report |
| `sw5-win` | Windows | HID con E/S overlapped y cancelación; registro con vista WOW64; claves OEM de DirectInput |
| `sw5-effectdrv` | Windows | La DLL: servidor COM `IDirectInputEffectDriver` y un hilo por volante con timer de alta resolución |
| `sw5-cli` | Windows | `sw5.exe`: diagnóstico, instalador con backup y restore, pruebas por DirectInput |
| `sw5-linux` | Linux | Servicio evdev → uinput → hidraw, instalador udev/systemd, pruebas |
| `xtask` | — | `cargo xtask dist`, `package` y `package-linux` |

### Qué mejora respecto del original

| Original | speedwheel5-rs |
|---|---|
| Un timer de 50 ms por efecto; el último que escribe gana | Un hilo cada 4 ms que **suma** todos los efectos activos |
| Spring, Damper, Inertia, Friction y Custom anunciados sin efecto | Custom implementado; condiciones aceptadas (sin motor de torsión no tienen sentido físico) |
| Envolvente, fase, ganancia global y pausa ignoradas | Implementadas |
| Crash en x64 (puntero truncado), sin locks, `TerminateThread` | Estado en `Arc<Mutex>`, `catch_unwind` en toda la frontera COM, descarga segura de la DLL |
| Cada actualización reiniciaba el efecto | Parámetros dinámicos |
| Build Debug y log a `c:\driverlog.txt` | Release con CRT estático; log solo si se configura |
| Instalador Wise en `%WINDIR%` con `regsvr32` mal usado | Program Files, CLSID propio en las dos vistas del registro, backup y restore |
| Solo Windows | Windows y Linux |

## Configuración

| Opción | Windows | Linux |
|---|---|---|
| Intensidad global (%) | `sw5 config --user --strength 70` | `sudo sw5-linux install --strength 70` |
| Nivel mínimo con el motor encendido | `--min-level 40` | `--min-level 40` |
| Curva de respuesta (× 100) | `--gamma 80` | `--gamma 80` |
| Intercambiar motores | `--swap-motors true` | `--swap-motors` |
| Período de mezcla (ms) | `--tick-ms 4` | `--tick-ms 4` |
| Log de diagnóstico | `--log-file RUTA --verbose true` | `--verbose` + `journalctl -u sw5-linux` |

En Windows, `--user` guarda solo para tu usuario y no pide administrador. Los
cambios se aplican la próxima vez que un juego abre el volante.

## Desarrollo

Requisitos: Rust estable (1.85 o posterior). En Windows hacen falta además las
MSVC Build Tools y `rustup target add i686-pc-windows-msvc`.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo xtask dist                         # Windows: dist\sw5.exe + DLL x64/x86
cargo xtask package                      # Windows: zip para distribuir
cargo xtask package-linux                # Linux: tar.gz estático
```

Herramientas de diagnóstico en Windows:

- `sw5 list`: capacidades HID.
- `sw5 raw 255 0`: motores en crudo.
- `sw5 selftest`: carga la DLL sin instalarla.
- `sw5 ffb-test all`: efectos por DirectInput.
- `sw5 status`: qué driver está usando Windows.

### Publicar una versión

1. Subí `version` en `Cargo.toml` (`[workspace.package]`) y hacé commit.
2. Creá y subí el tag: `git tag v0.1.0 && git push origin v0.1.0`.
3. El workflow `release.yml` compila el zip de Windows y el tar.gz de Linux,
   genera `SHA256SUMS.txt` y crea la release con todo.

## Pendiente

- Probar `sw5-linux` con el volante real, incluidos Steam, Proton y Steam Deck.
- Probar `sw5 uninstall` de punta a punta en una máquina real.
- Firma Authenticode: evita SmartScreen y bloqueos de anti-cheat.
- Ajustar cómo se traduce la fuerza constante a vibración en juegos de carreras.
- Vibración en juegos que solo usan XInput (puente ViGEm o x360ce).

## Licencia

MIT o Apache-2.0, a elección: ver [`LICENSE-MIT`](LICENSE-MIT) y
[`LICENSE-APACHE`](LICENSE-APACHE).

Proyecto no oficial, sin relación con Genius / KYE Systems, Arkino ni JessTech. Este
repositorio no contiene código ni binarios del driver original.
