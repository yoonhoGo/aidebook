# Aidebook product homepage

독립적인 정적 홈페이지. 앱 번들이나 Rust 빌드 없이 실행한다.

```sh
cd /Users/yoonho.go/workspace/aidebook-homepage/site
npm run dev
# http://127.0.0.1:4178
npm run build
# site/dist/를 정적 호스팅에 사용
```

- `index.html`: 한국어 제품 소개, 3개 자료 선택 데모, FAQ.
- `references.html`: 실제 공식 홈페이지 캡처와 디자인 판단. 로컬 검토용.
- `main.js`: 고정 예시 데이터를 선택하는 클라이언트 동작. 외부 요청/수집 없음.
- 시스템 폰트와 자체 SVG를 사용하므로 폰트 CDN 의존성 없음.
- 공개 빌드는 레퍼런스 보드/외부 서비스 캡처를 제외한다.
- 실제 앱 캡처가 아닌 설명용 UI 데모이며 Core/AI/외부 계정에 연결되지 않는다.
- `npm run dev`에는 Python 3, build에는 Node.js가 필요하다. 패키지 설치 불필요.

디자인 조사와 검증 기록: `../docs/HOMEPAGE_DESIGN.md`.
