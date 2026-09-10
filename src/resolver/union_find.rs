#[derive(Clone, Debug)]
pub struct UnionFind {
    parent: Vec<usize>,
    size: Vec<usize>,
}

impl UnionFind {
    pub fn new(length: usize) -> Self {
        Self {
            parent: (0..length).collect(),
            size: vec![1; length],
        }
    }

    pub fn find(&mut self, node: usize) -> usize {
        if self.parent[node] != node {
            self.parent[node] = self.find(self.parent[node]);
        }
        self.parent[node]
    }

    pub fn union(&mut self, left: usize, right: usize, sort_keys: &[Vec<u8>]) -> usize {
        let left = self.find(left);
        let right = self.find(right);
        if left == right {
            return left;
        }
        let (root, child) = if (&sort_keys[left], left) <= (&sort_keys[right], right) {
            (left, right)
        } else {
            (right, left)
        };
        self.parent[child] = root;
        self.size[root] += self.size[child];
        root
    }

    pub fn size(&mut self, node: usize) -> usize {
        let root = self.find(node);
        self.size[root]
    }
}
