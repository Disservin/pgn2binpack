import matplotlib.pyplot as plt
import numpy as np

data = np.loadtxt("score_pairs.txt")
original = data[:, 0]
rescored = data[:, 1]

fig, axes = plt.subplots(2, 2, figsize=(12, 12))

# Full plot
axes[0, 0].scatter(original, rescored, alpha=0.3, s=1)
axes[0, 0].axline((0, 0), slope=1, color="red", linestyle="--")
axes[0, 0].set_xlabel("Original Eval")
axes[0, 0].set_ylabel("Rescored Eval")
axes[0, 0].set_title("Full Range")
axes[0, 0].grid(True, alpha=0.3)

# Zoom: [-10000, 10000]
axes[0, 1].scatter(original, rescored, alpha=0.3, s=1)
axes[0, 1].axline((0, 0), slope=1, color="red", linestyle="--")
axes[0, 1].set_xlim(-10000, 10000)
axes[0, 1].set_ylim(-10000, 10000)
axes[0, 1].set_title("Zoom: ±10000")
axes[0, 1].grid(True, alpha=0.3)

# Zoom: [-1000, 1000]
axes[1, 0].scatter(original, rescored, alpha=0.3, s=1)
axes[1, 0].axline((0, 0), slope=1, color="red", linestyle="--")
axes[1, 0].set_xlim(-1000, 1000)
axes[1, 0].set_ylim(-1000, 1000)
axes[1, 0].set_title("Zoom: ±1000")
axes[1, 0].grid(True, alpha=0.3)

# Zoom: [-100, 100]
axes[1, 1].scatter(original, rescored, alpha=0.3, s=1)
axes[1, 1].axline((0, 0), slope=1, color="red", linestyle="--")
axes[1, 1].set_xlim(-100, 100)
axes[1, 1].set_ylim(-100, 100)
axes[1, 1].set_title("Zoom: ±100")
axes[1, 1].grid(True, alpha=0.3)

plt.tight_layout()
plt.savefig("eval_scatter_zoom.png", dpi=300, bbox_inches="tight")
plt.close()