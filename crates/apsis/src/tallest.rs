// SPDX-License-Identifier: GPL-3.0-only

//! [`Tallest`]: one of several contents, in a space as high as the tallest of them.
//!
//! The popup's settings view uses it so the details pane is sized once, for the longest
//! explanation, and doesn't change height while moving between rows.

use cosmic::iced::advanced::layout::{self, Layout};
use cosmic::iced::advanced::renderer;
use cosmic::iced::advanced::widget::{Tree, Widget};
use cosmic::iced::{Length, Rectangle, Size, mouse};
use cosmic::{Element, Renderer, Theme};

/// Lays out every child with the same limits and takes the size of the tallest (and widest);
/// draws only child `shown`. The children should be plain content (text): only the shown one
/// is drawn, and none of them gets events.
pub struct Tallest<'a, Message> {
    children: Vec<Element<'a, Message>>,
    shown: usize,
}

impl<'a, Message> Tallest<'a, Message> {
    pub fn new(children: Vec<Element<'a, Message>>, shown: usize) -> Self {
        Self { children, shown }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Tallest<'_, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn children(&self) -> Vec<Tree> {
        self.children.iter().map(Tree::new).collect()
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut self.children);
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let nodes: Vec<layout::Node> = self
            .children
            .iter_mut()
            .zip(&mut tree.children)
            .map(|(child, tree)| child.as_widget_mut().layout(tree, renderer, limits))
            .collect();
        let size = nodes.iter().fold(Size::ZERO, |size, node| {
            Size::new(
                size.width.max(node.size().width),
                size.height.max(node.size().height),
            )
        });
        layout::Node::with_children(limits.resolve(Length::Fill, Length::Shrink, size), nodes)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let shown = self
            .children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .nth(self.shown);
        if let Some(((child, tree), layout)) = shown {
            child
                .as_widget()
                .draw(tree, renderer, theme, style, layout, cursor, viewport);
        }
    }
}

impl<'a, Message: 'a> From<Tallest<'a, Message>> for Element<'a, Message> {
    fn from(tallest: Tallest<'a, Message>) -> Self {
        Element::new(tallest)
    }
}
