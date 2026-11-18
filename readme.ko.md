<h1 align="center">CollaboAddons</h1>

<p align="center"><a href="https://github.com/jkhda456/CollaboCore">CollaboCore</a> 에 쓸 만한 부가 애드온 모음</p>

<p align="center"><a href="readme.md">English</a> | <b>한국어</b></p>

[CollaboCore](https://github.com/jkhda456/CollaboCore) 는 올인원 WASM 샌드박스 머신입니다. 리눅스
커널이 네이티브 프로세스로 돌아가고, 에이전트는 root 셸을 쓰며, 폴더는 마운트되고, 네트워크는 하나의
정책으로 관리됩니다. 이 저장소는 그 위에 얹는 선택형 애드온을 제공합니다. 

하지만 그딴건 중요하지 않고 DOOM이 돌아갑니다. (오예!)

<p align="center">
  <img src="run_doom.png" width="600">
</p>

## 설치

`launcher --addons list` 는 이 저장소에 어떤 애드온이 있는지, 그중 이미 받아 둔 것은 무엇인지
보여줍니다. `launcher --addons get NAME...` 는 그것들을 런타임의 애드온 폴더로 내려받으며, 언제나
최신 버전입니다:

```
C:\Utils\CollaboCore>launcher.exe --addons list
C:\Utils\CollaboCore>launcher.exe --addons get doom
```

받은 애드온은 런처 옆의 `launcher.conf` 에 이름을 적어 주고, 평소처럼 런처를 실행하면 됩니다:

```ini
addon = doom
# 애드온 설정은 NAME:KEY=VALUE 로 적습니다 (이 줄은 addon = NAME 을 함께 뜻합니다)
# addon-config = doom:KEY=VALUE
```

기본값은 [`jkhda456/CollaboAddons`](https://github.com/jkhda456/CollaboAddons) 의 최신 릴리스를
가리키며, `launcher.conf` 의 `addons-repo = OWNER/REPO`(또는 API URL)로 다른 저장소를 지정할 수
있습니다.

나머지 — 엔진 옵션, 마운트, 네트워크 정책, 로그 — 는
[CollaboCore](https://github.com/jkhda456/CollaboCore) 의 `launcher.conf` 몫입니다. 주석이 달린 그
예시가 이 저장소에 들어 있습니다.
