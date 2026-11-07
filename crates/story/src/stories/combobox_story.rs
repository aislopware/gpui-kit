use gpui_kit::component::{
    ActiveTheme, Icon, IconName, IndexPath, Sizable as _,
    button::Button,
    button::ButtonVariants as _,
    combobox::*,
    h_flex,
    searchable_list::{
        SearchableGroup, SearchableListChange, SearchableListDelegate, SearchableListItem,
        SearchableVec,
    },
    v_flex,
};
use gpui_kit::{prelude::FluentBuilder as _, *};

use crate::section;

pub fn init(_: &mut App) {}

// MARK: Data

#[derive(Clone)]
struct FoodItem {
    label: SharedString,
    is_disabled: bool,
}

impl FoodItem {
    fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            is_disabled: false,
        }
    }

    fn disabled(mut self) -> Self {
        self.is_disabled = true;
        self
    }
}

impl SearchableListItem for FoodItem {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &SharedString {
        &self.label
    }

    fn disabled(&self) -> bool {
        self.is_disabled
    }
}

#[derive(Clone)]
struct Industry {
    label: SharedString,
    icon: IconName,
}

impl Industry {
    fn new(label: impl Into<SharedString>, icon: IconName) -> Self {
        Self {
            label: label.into(),
            icon,
        }
    }
}

impl SearchableListItem for Industry {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &SharedString {
        &self.label
    }

    fn render(
        &self,
        _window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::App,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::component::ActiveTheme as _;

        h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(
                Icon::new(self.icon.clone())
                    .small()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(gpui_kit::div().child(self.label.clone()))
    }
}

// MARK: Max2Delegate — allows at most 2 items to be selected simultaneously

/// Shadows the current selection indices so `is_item_enabled` can disable unselected items
/// when the capacity is reached, providing visual feedback without `current_selection` access.
struct Max2Delegate {
    items: SearchableVec<&'static str>,
    selected_indices: Vec<IndexPath>,
}

impl Max2Delegate {
    fn new(items: SearchableVec<&'static str>) -> Self {
        Self {
            items,
            selected_indices: Vec::new(),
        }
    }
}

impl SearchableListDelegate for Max2Delegate {
    type Item = &'static str;

    fn items_count(&self, section: usize) -> usize {
        self.items.items_count(section)
    }

    fn item(&self, ix: IndexPath) -> Option<&&'static str> {
        self.items.item(ix)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        &'static str: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.items.position(value)
    }

    fn is_item_enabled(&self, ix: IndexPath, _item: &&'static str, _cx: &App) -> bool {
        let at_capacity = self.selected_indices.len() >= 2;
        let is_selected = self.selected_indices.contains(&ix);

        !at_capacity || is_selected
    }

    fn on_will_change(
        &mut self,
        selection: &mut Vec<(IndexPath, &'static str)>,
        changes: &[SearchableListChange],
    ) {
        for change in changes {
            match change {
                SearchableListChange::Deselect { index } => {
                    selection.retain(|(ix, _)| ix != index);
                }
                SearchableListChange::Select { index } => {
                    if selection.len() < 2 {
                        if let Some(item) = self.item(*index) {
                            if !selection.iter().any(|(ix, _)| ix == index) {
                                selection.push((*index, *item));
                            }
                        }
                    }
                }
            }
        }

        // Keep the shadow in sync for is_item_enabled.
        self.selected_indices = selection.iter().map(|(ix, _)| *ix).collect();
    }
}

// MARK: PinnedDelegate — first two items always appear checked regardless of selection

struct PinnedDelegate(SearchableVec<&'static str>);

impl SearchableListDelegate for PinnedDelegate {
    type Item = &'static str;

    fn items_count(&self, section: usize) -> usize {
        self.0.items_count(section)
    }

    fn item(&self, ix: IndexPath) -> Option<&&'static str> {
        self.0.item(ix)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        &'static str: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.0.position(value)
    }

    fn is_item_enabled(&self, ix: IndexPath, _item: &&'static str, _cx: &App) -> bool {
        // Pinned items are non-interactive — their checked state is fixed.
        ix != IndexPath::new(0) && ix != IndexPath::new(1)
    }

    fn is_item_checked(
        &self,
        ix: IndexPath,
        _item: &&'static str,
        current_selection: &[(IndexPath, &'static str)],
        _cx: &App,
    ) -> bool {
        // First two items are always rendered checked (externally pinned),
        // regardless of what is in the normal selection.
        ix == IndexPath::new(0)
            || ix == IndexPath::new(1)
            || current_selection.iter().any(|(sel_ix, _)| sel_ix == &ix)
    }
}

// MARK: FeaturedDelegate — first item gets a "Featured" badge via render_item

struct FeaturedDelegate(SearchableVec<&'static str>);

impl SearchableListDelegate for FeaturedDelegate {
    type Item = &'static str;

    fn items_count(&self, section: usize) -> usize {
        self.0.items_count(section)
    }

    fn item(&self, ix: IndexPath) -> Option<&&'static str> {
        self.0.item(ix)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        &'static str: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.0.position(value)
    }

    fn render_item(
        &self,
        ix: IndexPath,
        item: &&'static str,
        checked: bool,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        if ix != IndexPath::new(0) {
            return None;
        }

        Some(
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .child(*item)
                .when(checked, |this| {
                    this.child(
                        Icon::new(IconName::Check)
                            .xsmall()
                            .text_color(cx.theme().muted_foreground),
                    )
                })
                .child(
                    div()
                        .rounded(cx.theme().radius.half())
                        .bg(cx.theme().primary)
                        .text_color(cx.theme().primary_foreground)
                        .px_1()
                        .text_xs()
                        .child("Featured"),
                )
                .into_any_element(),
        )
    }
}

// MARK: Story

pub struct ComboboxStory {
    // 01 basic single-select
    basic: Entity<ComboboxState<SearchableVec<&'static str>>>,
    // 02 basic multi-select
    basic_multi: Entity<ComboboxState<SearchableVec<&'static str>>>,
    // 03 grouped single-select
    grouped: Entity<ComboboxState<SearchableVec<SearchableGroup<FoodItem>>>>,
    // 04 disabled items (single)
    disabled_items: Entity<ComboboxState<SearchableVec<FoodItem>>>,
    // 05 item icon (single)
    with_icon: Entity<ComboboxState<SearchableVec<Industry>>>,
    // 06 custom check icon (single)
    custom_check: Entity<ComboboxState<SearchableVec<&'static str>>>,
    // 07 footer button (single)
    with_footer: Entity<ComboboxState<SearchableVec<String>>>,
    // 08 chips that wrap (multi)
    chips: Entity<ComboboxState<SearchableVec<&'static str>>>,
    // 09 on_will_change — max 2 items
    custom_max2: Entity<ComboboxState<Max2Delegate>>,
    // 10 is_item_checked — externally pinned items
    pinned: Entity<ComboboxState<PinnedDelegate>>,
    // 11 render_item delegate hook — custom row renderer
    featured: Entity<ComboboxState<FeaturedDelegate>>,
}

impl super::Story for ComboboxStory {
    fn title() -> &'static str {
        "Combobox"
    }

    fn description() -> &'static str {
        "A text input that filters a list of options as you type."
    }

    fn new_view(window: &mut Window, cx: &mut App) -> Entity<impl Render> {
        Self::view(window, cx)
    }
}

impl Focusable for ComboboxStory {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.basic.focus_handle(cx)
    }
}

const FRAMEWORKS: &[&str] = &["Next.js", "SvelteKit", "Nuxt.js", "Remix", "Astro"];

const MULTI_FRAMEWORKS: &[&str] = &[
    "React", "Nextjs", "Angular", "VueJS", "Django", "Astro", "Remix", "Svelte", "SolidJS", "Qwik",
];

fn food_groups() -> SearchableVec<SearchableGroup<FoodItem>> {
    SearchableVec::new(vec![
        SearchableGroup::new("Fruits").items(vec![
            FoodItem::new("Apples"),
            FoodItem::new("Bananas"),
            FoodItem::new("Cherries"),
        ]),
        SearchableGroup::new("Vegetables").items(vec![
            FoodItem::new("Carrots"),
            FoodItem::new("Broccoli").disabled(),
            FoodItem::new("Spinach"),
        ]),
        SearchableGroup::new("Beverages").items(vec![
            FoodItem::new("Tea"),
            FoodItem::new("Coffee").disabled(),
            FoodItem::new("Juice"),
        ]),
    ])
}

fn industries() -> SearchableVec<Industry> {
    SearchableVec::new(vec![
        Industry::new("Information Technology", IconName::Cpu),
        Industry::new("Healthcare", IconName::Heart),
        Industry::new("Finance", IconName::Globe),
        Industry::new("Education", IconName::BookOpen),
        Industry::new("Entertainment", IconName::Star),
    ])
}

impl ComboboxStory {
    fn new(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let basic = cx.new(|cx| {
            ComboboxState::new(SearchableVec::new(FRAMEWORKS.to_vec()), vec![], window, cx)
        });

        let basic_multi = cx.new(|cx| {
            ComboboxState::new(SearchableVec::new(FRAMEWORKS.to_vec()), vec![], window, cx)
                .multiple(true)
        });

        let grouped =
            cx.new(|cx| ComboboxState::new(food_groups(), vec![IndexPath::default()], window, cx));

        let disabled_items = cx.new(|cx| {
            let items = SearchableVec::new(vec![
                FoodItem::new("Apples"),
                FoodItem::new("Bananas").disabled(),
                FoodItem::new("Cherries"),
                FoodItem::new("Carrots"),
                FoodItem::new("Broccoli").disabled(),
            ]);
            ComboboxState::new(items, vec![], window, cx)
        });

        let with_icon = cx.new(|cx| ComboboxState::new(industries(), vec![], window, cx));

        let custom_check = cx.new(|cx| {
            ComboboxState::new(SearchableVec::new(FRAMEWORKS.to_vec()), vec![], window, cx)
        });

        let with_footer = cx.new(|cx| {
            let items = SearchableVec::new(vec![
                "Harvard University".to_string(),
                "MIT".to_string(),
                "Stanford".to_string(),
                "Cambridge".to_string(),
            ]);
            ComboboxState::new(items, vec![IndexPath::default()], window, cx)
        });

        let chips = cx.new(|cx| {
            ComboboxState::new(
                SearchableVec::new(MULTI_FRAMEWORKS.to_vec()),
                vec![
                    IndexPath::new(0),
                    IndexPath::new(2),
                    IndexPath::new(5),
                    IndexPath::new(8),
                ],
                window,
                cx,
            )
            .multiple(true)
        });

        let custom_max2 = cx.new(|cx| {
            ComboboxState::new(
                Max2Delegate::new(SearchableVec::new(MULTI_FRAMEWORKS.to_vec())),
                vec![],
                window,
                cx,
            )
            .multiple(true)
        });

        let pinned = cx.new(|cx| {
            ComboboxState::new(
                PinnedDelegate(SearchableVec::new(FRAMEWORKS.to_vec())),
                vec![],
                window,
                cx,
            )
        });

        let featured = cx.new(|cx| {
            ComboboxState::new(
                FeaturedDelegate(SearchableVec::new(FRAMEWORKS.to_vec())),
                vec![],
                window,
                cx,
            )
        });

        cx.new(|_| Self {
            basic,
            basic_multi,
            grouped,
            disabled_items,
            with_icon,
            custom_check,
            with_footer,
            chips,
            custom_max2,
            pinned,
            featured,
        })
    }

    pub fn view(window: &mut Window, cx: &mut App) -> Entity<Self> {
        Self::new(window, cx)
    }
}

impl Render for ComboboxStory {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .gap_4()
            .child(
                section("Default")
                    .w(px(280.))
                    .description("Type to filter, then choose one option.")
                    .child(
                        Combobox::new(&self.basic)
                            .placeholder("Select framework...")
                            .w(px(280.)),
                    ),
            )
            .child(
                section("Multiple")
                    .w(px(280.))
                    .description("Select more than one option; each shows as a chip.")
                    .child(
                        Combobox::new(&self.basic_multi)
                            .placeholder("Select frameworks...")
                            .w(px(280.)),
                    ),
            )
            .child(
                section("Groups")
                    .w(px(280.))
                    .description("Organize results into groups.")
                    .child(
                        Combobox::new(&self.grouped)
                            .placeholder("Select item...")
                            .w(px(280.)),
                    ),
            )
            .child(
                section("Disabled items")
                    .w(px(280.))
                    .description("Keep unavailable options visible.")
                    .child(
                        Combobox::new(&self.disabled_items)
                            .placeholder("Select item...")
                            .w(px(280.)),
                    ),
            )
            .child(
                section("Icons")
                    .w(px(280.))
                    .description("Show icons in options.")
                    .child(
                        Combobox::new(&self.with_icon)
                            .placeholder("Select industry category")
                            .w(px(280.)),
                    ),
            )
            .child(
                section("Check icon")
                    .w(px(280.))
                    .description("Replace the default selection mark.")
                    .child(
                        Combobox::new(&self.custom_check)
                            .placeholder("Select framework...")
                            .check_icon(Icon::new(IconName::CircleCheck))
                            .w(px(280.)),
                    ),
            )
            .child(
                section("Footer")
                    .w(px(280.))
                    .description("Add an action below the option list.")
                    .child({
                        let state = self.with_footer.clone();
                        Combobox::new(&self.with_footer)
                            .placeholder("Select university")
                            .footer(move |_, cx| {
                                let state = state.clone();
                                Button::new("add-new")
                                    .ghost()
                                    .label("New university")
                                    .icon(Icon::new(IconName::Plus))
                                    .text_color(cx.theme().foreground)
                                    .w_full()
                                    .justify_start()
                                    .on_click(move |_, window, cx| {
                                        let search_query = state.read(cx).query(cx);
                                        let mut items = SearchableVec::new(vec![
                                            "Harvard University".to_string(),
                                            "MIT".to_string(),
                                            "Stanford".to_string(),
                                            "Cambridge".to_string(),
                                            search_query.to_string(),
                                        ]);

                                        drop(items.perform_search(&search_query, window, cx));

                                        state.update(cx, |state, cx| {
                                            state.set_items(items, window, cx);
                                        });
                                    })
                                    .into_any_element()
                            })
                            .w(px(280.))
                    }),
            )
            .child(
                section("Chips")
                    .w(px(280.))
                    .description(
                        "Chips wrap onto new lines. Backspace removes the last; \
                         Left moves onto them.",
                    )
                    .child(
                        Combobox::new(&self.chips)
                            .placeholder("Select frameworks")
                            .cleanable(true)
                            .w(px(280.)),
                    ),
            )
            .child(
                section("Maximum selections")
                    .w(px(280.))
                    .description("Limit how many items can be selected.")
                    .child(
                        Combobox::new(&self.custom_max2)
                            .placeholder("Select up to 2 frameworks")
                            .w(px(280.)),
                    ),
            )
            .child(
                section("Pinned items")
                    .w(px(280.))
                    .description("Keep required items selected.")
                    .child(
                        Combobox::new(&self.pinned)
                            .placeholder("Select framework...")
                            .w(px(280.)),
                    ),
            )
            .child(
                section("Rich items")
                    .w(px(280.))
                    .description("Render supporting content in option rows.")
                    .child(
                        Combobox::new(&self.featured)
                            .placeholder("Select framework...")
                            .w(px(280.)),
                    ),
            )
            .child(
                section("Values")
                    .description("Read selected values from each delegate.")
                    .w_full()
                    .child(
                        v_flex()
                            .gap_2()
                            .child(format!(
                                "basic: {:?}",
                                self.basic.read(cx).selected_values()
                            ))
                            .child(format!(
                                "grouped: {:?}",
                                self.grouped.read(cx).selected_values()
                            ))
                            .child(format!(
                                "chips: {:?}",
                                self.chips.read(cx).selected_values()
                            )),
                    ),
            )
    }
}
