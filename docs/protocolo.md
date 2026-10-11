# Protocolo y notas técnicas

## Hardware

Genius **Speed Wheel 5 Pro**. VID `0x1A5C` (Arkino Corporation), PID `0x3112`; se
conoce también un `0x3012`. Datos leídos del dispositivo con `sw5 list`:

| | |
|---|---|
| Top-level usage | Generic Desktop (0x01) / Joystick (0x04) |
| Input report | 9 bytes: Report ID 0, Y (8 bits), X (8 bits), hat (4 bits), 11 botones |
| Output report | 5 bytes: Report ID 0 y 4 campos de 8 bits en la página LED (0x08), usages 0x43–0x46 |
| Feature reports | ninguno |

### Output report: los motores

```text
byte 0   byte 1   byte 2   byte 3          byte 4
 00       00      flags    A (0..255)      B (0..255)
                           motor fuerte    motor suave
```

- Se confirmó con el volante en la mano que hay **dos motores de vibración**: el
  canal A es el más fuerte y grave. No hay motor de torsión.
- `00 00 00 00 00` apaga los dos motores.
- Con `flags = 0` funciona. El driver original ponía el bit 1 (canal A) o el bit 0
  (canal B) cuando el canal llevaba más de una muestra seguida por debajo de 0x10.
  El significado exacto de esos bits no se confirmó.
- Es el mismo esquema que los rumble pads JessTech/Saitek que Linux maneja con
  `hid-pl`.

## Windows: cómo DirectInput elige el effect driver

Para un dispositivo HID, DirectInput lee el tipo OEM en
`...\MediaProperties\PrivateProperties\Joystick\OEM\VID_1A5C&PID_3112`
y carga el COM de `OEMForceFeedback\CLSID` dentro del proceso del juego.

- Lee **primero HKCU y después HKLM**. El instalador original (Wise, 2008)
  escribió las dos copias, así que `sw5 install` actualiza y respalda ambas.
- La ACL de esa copia en HKCU no da `WRITE_DAC` ni `WRITE_OWNER`. Si se abre con
  `KEY_ALL_ACCESS`, falla con acceso denegado incluso como administrador.
- `OEMForceFeedback\Effects\{GUID}\Attributes` es un `DIEFFECTATTRIBUTES`. El
  `dwEffectId` (0x101–0x10C) es lo que el driver recibe en `DownloadEffect`. El
  original declaraba `dwDynamicParams = 0`, lo que obligaba a reiniciar el efecto en
  cada actualización. `sw5` declara como dinámicos todos los parámetros salvo los ejes.
- DirectInput le pasa al driver la ruta exacta del dispositivo en
  `DIHIDFFINITINFO.pwszDeviceInterface`, dentro de `DeviceID`.
- DirectInput descarga la DLL con `FreeLibrary` por su cuenta. Por eso el hilo del
  dispositivo fija el módulo y sale con `FreeLibraryAndExitThread`.

## Linux

El kernel maneja el volante con `hid-generic`, sin force feedback. `sw5-linux`:

1. Toma el `/dev/input/eventN` real en modo exclusivo (`EVIOCGRAB`).
2. Crea un dispositivo uinput con los mismos ejes, botones e IDs, que además
   anuncia `FF_RUMBLE`, `FF_PERIODIC`, `FF_CONSTANT`, `FF_RAMP`, las condiciones
   y `FF_GAIN`.
3. Traduce los efectos que suben los juegos a `sw5_core::Effect`. `FF_RUMBLE`
   corresponde exactamente a los dos motores.
4. Mezcla cada 4 ms y escribe el report de 5 bytes en `/dev/hidrawN`.

La regla de udev (`71-sw5-wheel.rules`) oculta el dispositivo real a los usuarios.
Lo hace después de `70-uaccess` y antes de `73-seat-late`, para que los juegos vean
solo el virtual.

## Driver original (JessTech FFdriver 1.2.0.2)

Análisis estático de `jsdriver32/64.dll`. Lo más relevante para la reescritura:

| Problema | Consecuencia |
|---|---|
| Un `timeSetEvent` de 50 ms por efecto, sin mezcla | 20 Hz; el último efecto que escribe gana y parar uno corta todos |
| Spring, Damper, Inertia, Friction y Custom solo anunciados | Los juegos los crean y no pasa nada |
| Constant con `fabs`, Ramp y periódicos con offset 127 | Codificaciones incoherentes entre efectos |
| Lectura de `rglDirection[1]` con 1 eje | Con un volante, los periódicos salen invertidos |
| x64: `dwUser` del timer truncado a 32 bits | Crash del juego si el heap está por encima de 4 GB |
| Sin locks entre el hilo del juego y el timer | Use-after-free posible |
| Log en `c:\driverlog.txt` en cada línea | Jitter y un archivo que crece sin límite |
| Distribuido como build Debug | Más lento y con comportamiento que depende del heap de depuración |
