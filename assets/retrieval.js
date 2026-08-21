function checkSendChainQuiz() {
  const answers = {
    q1: "p1-ok",
    q2: "ws-echo",
    q3: "temporary-anchor"
  };
  let score = 0;
  for (const [name, expected] of Object.entries(answers)) {
    const chosen = document.querySelector(`input[name="${name}"]:checked`);
    if (chosen && chosen.value === expected) score += 1;
  }
  const feedback = document.getElementById("quiz-feedback");
  feedback.className = `feedback ${score === 3 ? "ok" : "no"}`;
  feedback.textContent = score === 3
    ? "3/3：你已经抓住主线。下一步可以沿源码自己追一次。"
    : `${score}/3：先记住三个锚点：P1 Ok 才出 sending、WS echo 才是 sent 的起点、temporaryId 用来对账。`;
}
