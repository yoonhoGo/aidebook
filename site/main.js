const content = document.querySelector('#evidence-content');
const examples = {
  decision: { title: '제품 방향 노트', path: 'Obsidian / 프로젝트 / 방향.md', quote: '첫 연결에서는 자료를 읽고, 앱 안에 기억을 남기는 것부터 시작한다.', reason: '새 기능을 설계할 때도 원본을 보존한다는 제품 원칙을 이어가기 위해.' },
  issue: { title: '연결 범위 검토 #42', path: 'GitHub / atlas / issues / 42', quote: '사용자가 선택한 저장소의 이슈와 댓글만 읽는다. 연결 범위를 자동으로 넓히지 않는다.', reason: '연결 설정 화면에서 접근 범위를 분명히 보여주고, 사용자가 직접 선택하게 하기 위해.' },
  review: { title: '온보딩 설계', path: 'Jira / ATLAS-18', quote: '첫 단계에서 읽을 프로젝트를 고르고, 연결 후에도 범위를 확인할 수 있어야 한다.', reason: '온보딩을 단순하게 만들면서도 어떤 자료가 읽히는지 사용자가 알 수 있도록.' }
};
document.querySelectorAll('[data-context]').forEach(button => {
  button.addEventListener('click', () => {
    const item = examples[button.dataset.context];
    document.querySelectorAll('[data-context]').forEach(node => node.setAttribute('aria-pressed', String(node === button)));
    content.innerHTML = `<h3>${item.title}</h3><p class="evidence-path">${item.path}</p><blockquote>“${item.quote}”</blockquote><span class="sidebar-title">이 기억을 남긴 이유</span><p>${item.reason}</p><div class="evidence-meta"><span>연결 방식</span><strong>명시적인 근거 참조</strong></div>`;
  });
});
