## scene-change-cadence-instability

ここではVapourSynth版IVTCやTFM + TDecimateでの間引き失敗例を記している。
30i 3:2プルダウンソースのフレームを1枚ずつコマ送りし、各フレームにおけるインターレース縞の有無を目視で記録した。

P: 目視で縞が見つからないフレーム。動きが少ないインターレースフレームの可能性もあるのでプログレッシブであることが確定したわけではない。
I: 目視で縞が明確に見つかるインターレースフレーム。
SC: シーンチェンジ
D: 縞は見られないが前のフレームと内容が完全に同じフレーム。
  ネット配信の24pソースを確認するとこのフレームは本来存在しないものだった。
  VapourSynth版IVTCやTIVTC、VIVTCの間引きでもダブりが残ったが、
  madVR内蔵IVTCの場合、SC後一瞬`unknown cadence` が表示されるが間引き後にダブりは発生していなかった。しかしソースが非公開なためロジックの確認ができない。

### OP

```
PPPII
PPPI[SC]P
PIIPI
PPPII
PPPII
PPPII
```

### ED

```
PPPII[SC]
PPIIP
PDPII
PPPII
PPPII
PPPII
```
